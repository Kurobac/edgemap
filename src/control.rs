use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags};
use nix::sys::socket::{
    accept4, bind, connect, listen, recv, send, socket, AddressFamily, Backlog, MsgFlags, SockFlag,
    SockType, UnixAddr,
};

use crate::config::ActiveConfig;

pub const LOCK_FILE_NAME: &str = "daemon.lock";
pub const SOCKET_FILE_NAME: &str = "control.sock";
pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_PACKET_SIZE: usize = 72 * 1024;
pub const MAX_CONTROL_CLIENTS: usize = 16;

const LISTENER_TOKEN: u64 = u64::MAX;
const MAX_ACCEPTS_PER_WAKE: usize = 16;
const MAX_CONFIG_SOURCE_SIZE: usize = 4096;
const SWITCH_CONFIG_PREFIX: &[u8] = b"switch-config\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlState {
    pub uhid_ready: bool,
    pub needs_config: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlRequest {
    SwitchConfig(ActiveConfig),
}

impl ControlRequest {
    fn ok_packet(&self) -> &'static [u8] {
        match self {
            Self::SwitchConfig(_) => b"ok switch-config",
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::SwitchConfig(config) => {
                let mut packet = Vec::with_capacity(
                    SWITCH_CONFIG_PREFIX.len() + config.source().len() + 1 + config.content().len(),
                );
                packet.extend_from_slice(SWITCH_CONFIG_PREFIX);
                packet.extend_from_slice(config.source().as_bytes());
                packet.push(0);
                packet.extend_from_slice(config.content().as_bytes());
                packet
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerPacket {
    Hello(ControlState),
    State(ControlState),
    OkSwitchConfig,
    Error { code: String, message: String },
}

#[derive(Debug)]
pub struct PendingRequest {
    pub client: RawFd,
    pub request: ControlRequest,
}

fn bool_digit(value: bool) -> char {
    if value {
        '1'
    } else {
        '0'
    }
}

fn hello_packet(state: ControlState) -> Vec<u8> {
    format!(
        "hello version={PROTOCOL_VERSION} uhid_ready={} needs_config={}",
        bool_digit(state.uhid_ready),
        bool_digit(state.needs_config)
    )
    .into_bytes()
}

fn state_packet(state: ControlState) -> Vec<u8> {
    format!(
        "state uhid_ready={} needs_config={}",
        bool_digit(state.uhid_ready),
        bool_digit(state.needs_config)
    )
    .into_bytes()
}

fn parse_state_fields(input: &str) -> Result<ControlState, String> {
    let mut fields = input.split_ascii_whitespace();
    let ready = fields
        .next()
        .and_then(|field| field.strip_prefix("uhid_ready="))
        .ok_or_else(|| "missing uhid_ready field".to_string())?;
    let needs = fields
        .next()
        .and_then(|field| field.strip_prefix("needs_config="))
        .ok_or_else(|| "missing needs_config field".to_string())?;
    if fields.next().is_some() {
        return Err("unexpected state fields".to_string());
    }
    let parse_bool = |value: &str| match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(format!("invalid boolean value {value:?}")),
    };
    Ok(ControlState {
        uhid_ready: parse_bool(ready)?,
        needs_config: parse_bool(needs)?,
    })
}

pub fn parse_server_packet(packet: &[u8]) -> Result<ServerPacket, String> {
    let text = std::str::from_utf8(packet).map_err(|_| "server packet is not UTF-8".to_string())?;
    if let Some(fields) = text.strip_prefix("hello ") {
        let fields = fields
            .strip_prefix(&format!("version={PROTOCOL_VERSION} "))
            .ok_or_else(|| "unsupported control protocol version".to_string())?;
        return parse_state_fields(fields).map(ServerPacket::Hello);
    }
    if let Some(fields) = text.strip_prefix("state ") {
        return parse_state_fields(fields).map(ServerPacket::State);
    }
    if text == "ok switch-config" {
        return Ok(ServerPacket::OkSwitchConfig);
    }
    if let Some(error) = text.strip_prefix("error ") {
        let (code, message) = error
            .split_once(' ')
            .ok_or_else(|| "malformed error packet".to_string())?;
        if code.is_empty() || message.is_empty() {
            return Err("malformed error packet".to_string());
        }
        return Ok(ServerPacket::Error {
            code: code.to_string(),
            message: message.to_string(),
        });
    }
    Err(format!("unknown server packet: {text:?}"))
}

fn parse_request(packet: &[u8]) -> Result<ControlRequest, String> {
    if let Some(payload) = packet.strip_prefix(SWITCH_CONFIG_PREFIX) {
        let separator = payload
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| "switch-config source delimiter is missing".to_string())?;
        let (source, content_with_separator) = payload.split_at(separator);
        if source.is_empty() {
            return Err("switch-config source is empty".to_string());
        }
        if source.len() > MAX_CONFIG_SOURCE_SIZE {
            return Err("switch-config source exceeds size limit".to_string());
        }
        let source = std::str::from_utf8(source)
            .map_err(|_| "switch-config source is not UTF-8".to_string())?;
        let content = std::str::from_utf8(&content_with_separator[1..])
            .map_err(|_| "switch-config content is not UTF-8".to_string())?;
        let active_config = ActiveConfig::from_content(source.to_string(), content.to_string())?;
        return Ok(ControlRequest::SwitchConfig(active_config));
    }
    let command = std::str::from_utf8(packet).map_err(|_| "request is not UTF-8".to_string())?;
    Err(format!("unknown command: {command:?}"))
}

fn send_packet(fd: RawFd, packet: &[u8]) -> io::Result<()> {
    if packet.len() > MAX_PACKET_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "control packet exceeds size limit",
        ));
    }
    match send(fd, packet, MsgFlags::MSG_NOSIGNAL | MsgFlags::MSG_DONTWAIT) {
        Ok(written) if written == packet.len() => Ok(()),
        Ok(written) => Err(io::Error::new(
            io::ErrorKind::WriteZero,
            format!("short seqpacket write: {written} of {}", packet.len()),
        )),
        Err(error) => Err(error.into()),
    }
}

enum RecvPacket {
    Data(Vec<u8>),
    Closed,
    WouldBlock,
}

fn recv_packet(fd: RawFd) -> io::Result<RecvPacket> {
    let mut buffer = vec![0u8; MAX_PACKET_SIZE];
    match recv(
        fd,
        &mut buffer,
        MsgFlags::MSG_DONTWAIT | MsgFlags::MSG_TRUNC,
    ) {
        Ok(0) => Ok(RecvPacket::Closed),
        Ok(size) if size > buffer.len() => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control packet exceeds size limit",
        )),
        Ok(size) => {
            buffer.truncate(size);
            Ok(RecvPacket::Data(buffer))
        }
        Err(nix::errno::Errno::EAGAIN) => Ok(RecvPacket::WouldBlock),
        Err(error) => Err(error.into()),
    }
}

pub struct ControlServer {
    listener: OwnedFd,
    epoll: Epoll,
    clients: HashMap<RawFd, OwnedFd>,
    state: ControlState,
    socket_path: PathBuf,
}

impl ControlServer {
    pub fn bind(runtime_dir: &Path, state: ControlState) -> io::Result<Self> {
        std::fs::create_dir_all(runtime_dir)?;
        let socket_path = runtime_dir.join(SOCKET_FILE_NAME);
        match std::fs::remove_file(&socket_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }

        let listener = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
            None,
        )?;
        let address = UnixAddr::new(&socket_path)?;
        bind(listener.as_raw_fd(), &address)?;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o666))?;
        listen(&listener, Backlog::new(16)?)?;

        let epoll = Epoll::new(EpollCreateFlags::EPOLL_CLOEXEC)?;
        epoll.add(
            listener.as_fd(),
            EpollEvent::new(EpollFlags::EPOLLIN, LISTENER_TOKEN),
        )?;
        Ok(Self {
            listener,
            epoll,
            clients: HashMap::new(),
            state,
            socket_path,
        })
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.epoll.0.as_fd()
    }

    pub fn state(&self) -> ControlState {
        self.state
    }

    pub fn set_state(&mut self, state: ControlState) {
        if self.state == state {
            return;
        }
        self.state = state;
        let packet = state_packet(state);
        let failed: Vec<_> = self
            .clients
            .keys()
            .copied()
            .filter(|fd| send_packet(*fd, &packet).is_err())
            .collect();
        for fd in failed {
            self.clients.remove(&fd);
        }
    }

    pub fn drain_requests(&mut self) -> io::Result<Vec<PendingRequest>> {
        let mut events = [EpollEvent::empty(); 32];
        let count = self.epoll.wait(&mut events, 0u8)?;
        let mut requests = Vec::new();
        let mut remove = Vec::new();

        for event in events.iter().take(count) {
            let token = event.data();
            if token == LISTENER_TOKEN {
                self.accept_clients()?;
                continue;
            }
            let fd = token as RawFd;
            if !self.clients.contains_key(&fd) {
                continue;
            }
            let flags = event.events();
            if flags.intersects(EpollFlags::EPOLLERR | EpollFlags::EPOLLHUP) {
                remove.push(fd);
                continue;
            }
            if !flags.contains(EpollFlags::EPOLLIN) {
                continue;
            }
            if !requests.is_empty() {
                // Keep control work bounded so a client flood cannot starve
                // hidraw and UHID handling in the outer event loop.
                continue;
            }

            match recv_packet(fd) {
                Ok(RecvPacket::Data(packet)) => {
                    if std::str::from_utf8(&packet).is_err() {
                        self.reply_error(fd, "protocol", "request is not UTF-8");
                        remove.push(fd);
                        continue;
                    }
                    match parse_request(&packet) {
                        Ok(request) => requests.push(PendingRequest {
                            client: fd,
                            request,
                        }),
                        Err(message) => {
                            self.reply_error(fd, "protocol", &message);
                        }
                    }
                }
                Ok(RecvPacket::Closed) => remove.push(fd),
                Ok(RecvPacket::WouldBlock) => {}
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    self.reply_error(fd, "protocol", &error.to_string());
                    remove.push(fd);
                }
                Err(_) => remove.push(fd),
            }
        }

        for fd in remove {
            self.clients.remove(&fd);
        }
        Ok(requests)
    }

    pub fn reply_ok(&mut self, client: RawFd, request: &ControlRequest) {
        self.reply(client, request.ok_packet());
    }

    pub fn reply_error(&mut self, client: RawFd, code: &str, message: &str) {
        let packet = format!("error {code} {message}");
        self.reply(client, packet.as_bytes());
    }

    fn reply(&mut self, client: RawFd, packet: &[u8]) {
        if let Err(error) = send_packet(client, packet) {
            log::debug!(
                "control client disconnected after reply failure: fd={client}, error={error}"
            );
            self.clients.remove(&client);
        }
    }

    fn accept_clients(&mut self) -> io::Result<()> {
        for _ in 0..MAX_ACCEPTS_PER_WAKE {
            let fd = match accept4(
                self.listener.as_raw_fd(),
                SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
            ) {
                Ok(fd) => fd,
                Err(nix::errno::Errno::EAGAIN) => return Ok(()),
                Err(nix::errno::Errno::EINTR) => continue,
                Err(error) => return Err(error.into()),
            };
            let client = unsafe { OwnedFd::from_raw_fd(fd) };
            if self.clients.len() >= MAX_CONTROL_CLIENTS {
                let _ = send_packet(fd, b"error busy control client limit reached");
                continue;
            }
            if send_packet(fd, &hello_packet(self.state)).is_err() {
                continue;
            }
            if let Err(error) = self.epoll.add(
                client.as_fd(),
                EpollEvent::new(
                    EpollFlags::EPOLLIN | EpollFlags::EPOLLERR | EpollFlags::EPOLLHUP,
                    fd as u64,
                ),
            ) {
                log::warn!("control client rejected after epoll registration failure: fd={fd}, error={error}");
                continue;
            }
            self.clients.insert(fd, client);
        }
        Ok(())
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

pub struct ControlClient {
    fd: OwnedFd,
}

impl ControlClient {
    pub fn connect(path: &Path) -> io::Result<Self> {
        let fd = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
            None,
        )?;
        let address = UnixAddr::new(path)?;
        connect(fd.as_raw_fd(), &address)?;
        Ok(Self { fd })
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    pub fn send_request(&self, request: &ControlRequest) -> io::Result<()> {
        send_packet(self.fd.as_raw_fd(), &request.encode())
    }

    pub fn receive(&self) -> io::Result<Option<ServerPacket>> {
        match recv_packet(self.fd.as_raw_fd())? {
            RecvPacket::Data(packet) => parse_server_packet(&packet)
                .map(Some)
                .map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message)),
            RecvPacket::Closed => Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "dseuhid control socket closed",
            )),
            RecvPacket::WouldBlock => Ok(None),
        }
    }
}

pub struct DaemonLock {
    _file: File,
}

impl DaemonLock {
    pub fn acquire(runtime_dir: &Path) -> io::Result<Self> {
        Self::acquire_named(runtime_dir, LOCK_FILE_NAME, "dseuhid")
    }

    pub fn acquire_named(
        runtime_dir: &Path,
        file_name: &str,
        process_name: &str,
    ) -> io::Result<Self> {
        std::fs::create_dir_all(runtime_dir)?;
        let path = runtime_dir.join(file_name);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o644)
            .open(&path)?;

        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                let mut owner = String::new();
                let _ = file.seek(SeekFrom::Start(0));
                let _ = file.read_to_string(&mut owner);
                let owner = owner.trim();
                let detail = if owner.is_empty() {
                    format!("another {process_name} instance holds the daemon lock")
                } else {
                    format!("another {process_name} instance holds the daemon lock (PID {owner})")
                };
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, detail));
            }
            return Err(error);
        }

        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "{}", std::process::id())?;
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_config(source: &str, content: &str) -> ActiveConfig {
        ActiveConfig::from_content(source.to_string(), content.to_string()).unwrap()
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("dseuhid-{name}-{}-{unique}", std::process::id()))
    }

    #[test]
    fn daemon_lock_is_exclusive_and_released_on_drop() {
        let dir = temp_dir("lock");
        let first = DaemonLock::acquire(&dir).unwrap();
        let error = DaemonLock::acquire(&dir).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(error.to_string().contains(&std::process::id().to_string()));

        drop(first);
        let second = DaemonLock::acquire(&dir).unwrap();
        drop(second);
        let named = DaemonLock::acquire_named(&dir, "edgemap.lock", "edgemap daemon").unwrap();
        assert!(dir.join("edgemap.lock").exists());
        drop(named);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stale_lock_file_does_not_block_startup() {
        let dir = temp_dir("stale-lock");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LOCK_FILE_NAME), "999999\n").unwrap();

        let lock = DaemonLock::acquire(&dir).unwrap();
        let content = std::fs::read_to_string(dir.join(LOCK_FILE_NAME)).unwrap();
        assert_eq!(content.trim(), std::process::id().to_string());

        drop(lock);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn protocol_preserves_switch_config_source_and_content_and_parses_state() {
        let request = ControlRequest::SwitchConfig(active_config(
            "/tmp/a path\nconfig.toml",
            "version = 2\n[cross]\nremap = \"circle\"\n",
        ));
        assert_eq!(parse_request(&request.encode()), Ok(request));
        assert_eq!(
            parse_server_packet(b"hello version=1 uhid_ready=1 needs_config=0"),
            Ok(ServerPacket::Hello(ControlState {
                uhid_ready: true,
                needs_config: false,
            }))
        );
        assert!(parse_request(b"switch-config\0\0content").is_err());
        assert!(parse_request(b"switch-config\0source-without-delimiter").is_err());
        assert!(parse_request(b"reload").is_err());
        assert!(parse_request(&[0xff]).is_err());
        assert!(parse_server_packet(b"hello version=2 uhid_ready=1 needs_config=0").is_err());
    }

    #[test]
    fn switch_config_protocol_enforces_source_content_and_packet_limits() {
        let maximum = ControlRequest::SwitchConfig(active_config(
            &"s".repeat(MAX_CONFIG_SOURCE_SIZE),
            &"x".repeat(crate::config::MAX_CONFIG_FILE_SIZE),
        ));
        let packet = maximum.encode();
        assert!(packet.len() <= MAX_PACKET_SIZE);
        assert_eq!(parse_request(&packet), Ok(maximum));

        let oversized_source = ControlRequest::SwitchConfig(active_config(
            &"s".repeat(MAX_CONFIG_SOURCE_SIZE + 1),
            "version = 2\n",
        ));
        assert!(parse_request(&oversized_source.encode()).is_err());
        assert!(ActiveConfig::from_content(
            "source".to_string(),
            "x".repeat(crate::config::MAX_CONFIG_FILE_SIZE + 1),
        )
        .is_err());

        let mut invalid_source = SWITCH_CONFIG_PREFIX.to_vec();
        invalid_source.extend_from_slice(&[0xff, 0, b'x']);
        assert!(parse_request(&invalid_source).is_err());

        let mut invalid_content = SWITCH_CONFIG_PREFIX.to_vec();
        invalid_content.extend_from_slice(b"source\0");
        invalid_content.push(0xff);
        assert!(parse_request(&invalid_content).is_err());
    }

    #[test]
    fn seqpacket_server_sends_hello_ack_error_and_state() {
        let dir = temp_dir("socket");
        let initial = ControlState {
            uhid_ready: false,
            needs_config: true,
        };
        let mut server = ControlServer::bind(&dir, initial).unwrap();
        let outer_epoll = Epoll::new(EpollCreateFlags::EPOLL_CLOEXEC).unwrap();
        outer_epoll
            .add(server.as_fd(), EpollEvent::new(EpollFlags::EPOLLIN, 1))
            .unwrap();
        let client = ControlClient::connect(&dir.join(SOCKET_FILE_NAME)).unwrap();
        let mut outer_events = [EpollEvent::empty(); 1];
        assert_eq!(outer_epoll.wait(&mut outer_events, 1000u16).unwrap(), 1);

        assert!(server.drain_requests().unwrap().is_empty());
        assert_eq!(
            client.receive().unwrap(),
            Some(ServerPacket::Hello(initial))
        );

        let first_request =
            ControlRequest::SwitchConfig(active_config("/tmp/one.toml", "version = 2\n"));
        client.send_request(&first_request).unwrap();
        let mut requests = server.drain_requests().unwrap();
        assert_eq!(requests.len(), 1);
        let pending = requests.pop().unwrap();
        assert_eq!(pending.request, first_request);
        server.reply_ok(pending.client, &pending.request);
        assert_eq!(
            client.receive().unwrap(),
            Some(ServerPacket::OkSwitchConfig)
        );

        client.send_request(&first_request).unwrap();
        let pending = server.drain_requests().unwrap().pop().unwrap();
        server.reply_error(pending.client, "not-ready", "UHID proxy is not ready");
        assert_eq!(
            client.receive().unwrap(),
            Some(ServerPacket::Error {
                code: "not-ready".to_string(),
                message: "UHID proxy is not ready".to_string(),
            })
        );

        let second = ControlClient::connect(&dir.join(SOCKET_FILE_NAME)).unwrap();
        assert!(server.drain_requests().unwrap().is_empty());
        assert_eq!(
            second.receive().unwrap(),
            Some(ServerPacket::Hello(initial))
        );

        let ready = ControlState {
            uhid_ready: true,
            needs_config: false,
        };
        server.set_state(ready);
        assert_eq!(client.receive().unwrap(), Some(ServerPacket::State(ready)));
        assert_eq!(second.receive().unwrap(), Some(ServerPacket::State(ready)));

        client.send_request(&first_request).unwrap();
        second
            .send_request(&ControlRequest::SwitchConfig(active_config(
                "/tmp/two.toml",
                "version = 2\n",
            )))
            .unwrap();
        let mut requests = server.drain_requests().unwrap();
        assert_eq!(requests.len(), 1);
        requests.extend(server.drain_requests().unwrap());
        requests.sort_by_key(|pending| pending.client);
        assert_eq!(requests.len(), 2);
        assert!(requests
            .iter()
            .any(|pending| pending.request == first_request));
        assert!(requests.iter().any(|pending| {
            pending.request
                == ControlRequest::SwitchConfig(active_config("/tmp/two.toml", "version = 2\n"))
        }));

        drop(client);
        drop(second);
        drop(server);
        assert!(!dir.join(SOCKET_FILE_NAME).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn disconnected_client_does_not_break_control_server() {
        let dir = temp_dir("disconnected-client");
        let initial = ControlState {
            uhid_ready: true,
            needs_config: false,
        };
        let mut server = ControlServer::bind(&dir, initial).unwrap();
        let client = ControlClient::connect(&dir.join(SOCKET_FILE_NAME)).unwrap();
        assert!(server.drain_requests().unwrap().is_empty());
        assert_eq!(
            client.receive().unwrap(),
            Some(ServerPacket::Hello(initial))
        );

        client
            .send_request(&ControlRequest::SwitchConfig(active_config(
                "/tmp/disconnected.toml",
                "version = 2\n",
            )))
            .unwrap();
        let pending = server.drain_requests().unwrap().pop().unwrap();
        drop(client);
        server.reply_ok(pending.client, &pending.request);

        let next = ControlClient::connect(&dir.join(SOCKET_FILE_NAME)).unwrap();
        assert!(server.drain_requests().unwrap().is_empty());
        assert_eq!(next.receive().unwrap(), Some(ServerPacket::Hello(initial)));

        drop(next);
        drop(server);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn control_server_limits_clients_without_affecting_existing_connections() {
        let dir = temp_dir("client-limit");
        let initial = ControlState {
            uhid_ready: true,
            needs_config: false,
        };
        let mut server = ControlServer::bind(&dir, initial).unwrap();
        let mut clients = Vec::new();

        for _ in 0..MAX_CONTROL_CLIENTS {
            let client = ControlClient::connect(&dir.join(SOCKET_FILE_NAME)).unwrap();
            assert!(server.drain_requests().unwrap().is_empty());
            assert_eq!(
                client.receive().unwrap(),
                Some(ServerPacket::Hello(initial))
            );
            clients.push(client);
        }
        assert_eq!(server.clients.len(), MAX_CONTROL_CLIENTS);

        let rejected = ControlClient::connect(&dir.join(SOCKET_FILE_NAME)).unwrap();
        assert!(server.drain_requests().unwrap().is_empty());
        assert_eq!(
            rejected.receive().unwrap(),
            Some(ServerPacket::Error {
                code: "busy".to_string(),
                message: "control client limit reached".to_string(),
            })
        );

        clients[0]
            .send_request(&ControlRequest::SwitchConfig(active_config(
                "/tmp/client-limit.toml",
                "version = 2\n",
            )))
            .unwrap();
        let pending = server.drain_requests().unwrap().pop().unwrap();
        server.reply_ok(pending.client, &pending.request);
        assert_eq!(
            clients[0].receive().unwrap(),
            Some(ServerPacket::OkSwitchConfig)
        );

        drop(rejected);
        drop(clients);
        drop(server);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
