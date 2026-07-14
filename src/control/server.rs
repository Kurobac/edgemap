use std::collections::HashMap;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags};
use nix::sys::socket::{
    accept4, bind, listen, socket, AddressFamily, Backlog, SockFlag, SockType, UnixAddr,
};

use super::protocol::{hello_packet, parse_request, state_packet, ControlRequest, ControlState};
use super::transport::{recv_packet, send_packet, RecvPacket};

pub const SOCKET_FILE_NAME: &str = "control.sock";
pub const MAX_CONTROL_CLIENTS: usize = 16;

const LISTENER_TOKEN: u64 = u64::MAX;
const MAX_ACCEPTS_PER_WAKE: usize = 16;

#[derive(Debug)]
pub struct PendingRequest {
    pub client: RawFd,
    pub request: ControlRequest,
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

    #[cfg(test)]
    pub(super) fn client_count(&self) -> usize {
        self.clients.len()
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
                        Err(message) => self.reply_error(fd, "protocol", &message),
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
