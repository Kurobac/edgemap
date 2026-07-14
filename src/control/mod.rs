mod client;
mod lock;
mod protocol;
mod server;
mod transport;

pub use client::ControlClient;
pub use lock::{DaemonLock, LOCK_FILE_NAME};
#[cfg(test)]
use protocol::{parse_request, MAX_CONFIG_SOURCE_SIZE, SWITCH_CONFIG_PREFIX};
pub use protocol::{
    parse_server_packet, ControlRequest, ControlState, ServerPacket, PROTOCOL_VERSION,
};
pub use server::{ControlServer, PendingRequest, MAX_CONTROL_CLIENTS, SOCKET_FILE_NAME};

#[cfg(test)]
use crate::config::ActiveConfig;
#[cfg(test)]
use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags};
#[cfg(test)]
use std::io;

pub const MAX_PACKET_SIZE: usize = 72 * 1024;
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
        assert_eq!(server.client_count(), MAX_CONTROL_CLIENTS);

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
