use std::path::Path;
use std::time::{Duration, Instant};

use dseuhid::{control, shutdown::ShutdownSignal};
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};

const CONTROL_SOCKET_PATH: &str = "/run/dseuhid/control.sock";
const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn drain_control_state(
    client: &control::ControlClient,
) -> Result<Option<control::ControlState>, String> {
    let mut latest = None;
    loop {
        match client.receive().map_err(|e| e.to_string())? {
            Some(control::ServerPacket::State(state)) => latest = Some(state),
            Some(packet) => return Err(format!("unexpected control packet: {packet:?}")),
            None => return Ok(latest),
        }
    }
}

pub(crate) enum DaemonRequestError {
    Shutdown,
    Failed(String),
}

pub(crate) fn send_daemon_control_request(
    client: &control::ControlClient,
    request: &control::ControlRequest,
    shutdown: &ShutdownSignal,
    state: &mut control::ControlState,
) -> Result<(), DaemonRequestError> {
    client
        .send_request(request)
        .map_err(|e| DaemonRequestError::Failed(e.to_string()))?;
    let deadline = Instant::now() + CONTROL_TIMEOUT;
    loop {
        if let Some(packet) = client
            .receive()
            .map_err(|e| DaemonRequestError::Failed(e.to_string()))?
        {
            match packet {
                control::ServerPacket::State(new_state) => *state = new_state,
                control::ServerPacket::OkSwitchConfig
                    if matches!(request, control::ControlRequest::SwitchConfig(_)) =>
                {
                    return Ok(())
                }
                control::ServerPacket::Error { code, message } => {
                    return Err(DaemonRequestError::Failed(format!("{code}: {message}")))
                }
                packet => {
                    return Err(DaemonRequestError::Failed(format!(
                        "unexpected control response: {packet:?}"
                    )))
                }
            }
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(DaemonRequestError::Failed(
                "timed out waiting for dseuhid control response".to_string(),
            ));
        }
        let timeout_ms = remaining.as_millis().min(u16::MAX as u128) as u16;
        let mut fds = [
            PollFd::new(
                client.as_fd(),
                PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP,
            ),
            PollFd::new(shutdown.as_fd(), PollFlags::POLLIN),
        ];
        match poll(&mut fds, PollTimeout::from(timeout_ms)) {
            Ok(0) => {
                return Err(DaemonRequestError::Failed(
                    "timed out waiting for dseuhid control response".to_string(),
                ))
            }
            Ok(_) => {
                let socket_events = fds[0].revents().unwrap_or(PollFlags::empty());
                let shutdown_events = fds[1].revents().unwrap_or(PollFlags::empty());
                if shutdown_events.contains(PollFlags::POLLIN) {
                    let _ = shutdown.consume();
                    return Err(DaemonRequestError::Shutdown);
                }
                if socket_events
                    .intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL)
                {
                    return Err(DaemonRequestError::Failed(
                        "dseuhid control socket disconnected".to_string(),
                    ));
                }
            }
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => {
                return Err(DaemonRequestError::Failed(format!(
                    "control socket poll failed: {e}"
                )))
            }
        }
    }
}

fn wait_for_control_packet(
    client: &control::ControlClient,
    deadline: Instant,
) -> Result<control::ServerPacket, String> {
    loop {
        if let Some(packet) = client.receive().map_err(|e| e.to_string())? {
            return Ok(packet);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("timed out waiting for dseuhid control response".to_string());
        }
        let timeout_ms = remaining.as_millis().min(u16::MAX as u128) as u16;
        let mut fds = [PollFd::new(
            client.as_fd(),
            PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP,
        )];
        match poll(&mut fds, PollTimeout::from(timeout_ms)) {
            Ok(0) => return Err("timed out waiting for dseuhid control response".to_string()),
            Ok(_) => {
                let events = fds[0].revents().unwrap_or(PollFlags::empty());
                if events.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL)
                {
                    return Err("dseuhid control socket disconnected".to_string());
                }
            }
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => return Err(format!("control socket poll failed: {e}")),
        }
    }
}

pub(crate) fn connect_control() -> Result<(control::ControlClient, control::ControlState), String> {
    let path = Path::new(CONTROL_SOCKET_PATH);
    let client = control::ControlClient::connect(path)
        .map_err(|e| format!("cannot connect to {CONTROL_SOCKET_PATH}: {e}"))?;
    let deadline = Instant::now() + CONTROL_TIMEOUT;
    match wait_for_control_packet(&client, deadline)? {
        control::ServerPacket::Hello(state) => Ok((client, state)),
        packet => Err(format!("expected control hello, received {packet:?}")),
    }
}

pub(crate) fn send_control_request(
    request: &control::ControlRequest,
) -> Result<control::ControlState, String> {
    let (client, mut state) = connect_control()?;
    client.send_request(request).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + CONTROL_TIMEOUT;
    loop {
        match wait_for_control_packet(&client, deadline)? {
            control::ServerPacket::State(new_state) => state = new_state,
            control::ServerPacket::OkSwitchConfig
                if matches!(request, control::ControlRequest::SwitchConfig(_)) =>
            {
                return Ok(state)
            }
            control::ServerPacket::OkHapticsDemo
                if matches!(request, control::ControlRequest::HapticsDemo) =>
            {
                return Ok(state)
            }
            control::ServerPacket::Error { code, message } => {
                return Err(format!("{code}: {message}"));
            }
            packet => return Err(format!("unexpected control response: {packet:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use control::{ControlClient, ControlRequest, ControlServer, ControlState, ServerPacket};
    use nix::sys::socket::{send, shutdown, MsgFlags, Shutdown};

    const INITIAL: ControlState = ControlState {
        uhid_ready: false,
        needs_config: true,
    };
    const READY: ControlState = ControlState {
        uhid_ready: true,
        needs_config: false,
    };

    fn with_connection(test: impl FnOnce(&mut ControlServer, &ControlClient)) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::path::PathBuf::from(format!(
            "/tmp/ecs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut server = ControlServer::bind(&dir, INITIAL).unwrap();
        let client = ControlClient::connect(&dir.join("control.sock")).unwrap();
        assert!(server.drain_requests().unwrap().is_empty());
        assert_eq!(
            wait_for_control_packet(&client, Instant::now() + CONTROL_TIMEOUT).unwrap(),
            ServerPacket::Hello(INITIAL)
        );
        test(&mut server, &client);
        drop(client);
        drop(server);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn request() -> ControlRequest {
        ControlRequest::SwitchConfig(
            dseuhid::config::ActiveConfig::from_content(
                "profile.toml".into(),
                "version = 2\n".into(),
            )
            .unwrap(),
        )
    }

    fn reply_to_request(
        server: &mut ControlServer,
        packets: &[&[u8]],
        disconnect: bool,
    ) -> std::os::fd::RawFd {
        let mut fds = [PollFd::new(server.as_fd(), PollFlags::POLLIN)];
        assert_eq!(
            poll(&mut fds, 1000u16).unwrap(),
            1,
            "client never sent its request"
        );
        let requests = server.drain_requests().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].request, request());
        let fd = requests[0].client;
        for packet in packets {
            assert_eq!(
                send(fd, packet, MsgFlags::MSG_NOSIGNAL).unwrap(),
                packet.len()
            );
        }
        if disconnect {
            shutdown(fd, Shutdown::Both).unwrap();
        }
        fd
    }

    #[test]
    fn daemon_request_waits_through_state_packets_for_ack() {
        with_connection(|server, client| {
            let shutdown = ShutdownSignal::new().unwrap();
            let mut state = INITIAL;
            let (completed, completion) = std::sync::mpsc::channel();
            std::thread::scope(|scope| {
                let responder = scope.spawn(move || {
                    let fd = reply_to_request(
                        server,
                        &[
                            b"state uhid_ready=1 needs_config=1",
                            b"state uhid_ready=1 needs_config=0",
                        ],
                        false,
                    );
                    assert!(
                        matches!(
                            completion.recv_timeout(Duration::from_millis(50)),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                        ),
                        "state updates completed the request before its ACK"
                    );
                    send(fd, b"ok switch-config", MsgFlags::MSG_NOSIGNAL).unwrap();
                });
                let result = send_daemon_control_request(client, &request(), &shutdown, &mut state);
                let _ = completed.send(());
                responder.join().unwrap();
                assert!(result.is_ok());
            });
            assert_eq!(state, READY);
        });
    }

    #[test]
    fn daemon_request_reports_error_unexpected_packet_and_disconnect() {
        for (packets, disconnect, expected) in [
            (
                vec![b"error validation-failed invalid config".as_slice()],
                false,
                "validation-failed: invalid config",
            ),
            (
                vec![b"hello version=1 uhid_ready=1 needs_config=0".as_slice()],
                false,
                "unexpected control response",
            ),
            (vec![], true, "control socket"),
        ] {
            with_connection(|server, client| {
                let shutdown = ShutdownSignal::new().unwrap();
                let mut state = INITIAL;
                std::thread::scope(|scope| {
                    let responder = scope.spawn(|| reply_to_request(server, &packets, disconnect));
                    let result =
                        send_daemon_control_request(client, &request(), &shutdown, &mut state);
                    responder.join().unwrap();
                    match result {
                        Err(DaemonRequestError::Failed(message)) => {
                            assert!(message.contains(expected), "{message}")
                        }
                        _ => panic!("request did not report the expected failure: {expected}"),
                    }
                });
                assert_eq!(state, INITIAL);
            });
        }
    }

    #[test]
    fn state_drain_keeps_latest_state_and_rejects_unsolicited_ack() {
        with_connection(|server, client| {
            assert_eq!(drain_control_state(client).unwrap(), None);
            server.set_state(ControlState {
                uhid_ready: true,
                needs_config: true,
            });
            server.set_state(READY);
            assert_eq!(drain_control_state(client).unwrap(), Some(READY));
            assert_eq!(drain_control_state(client).unwrap(), None);
            client.send_request(&request()).unwrap();
            let pending = server.drain_requests().unwrap().pop().unwrap();
            server.reply_ok(pending.client, &pending.request);
            assert!(drain_control_state(client)
                .unwrap_err()
                .contains("unexpected control packet"));
        });
    }

    #[test]
    fn silent_control_peer_times_out_without_advancing_state() {
        with_connection(|_, client| {
            for deadline in [Instant::now(), Instant::now() + Duration::from_millis(10)] {
                assert!(wait_for_control_packet(client, deadline)
                    .unwrap_err()
                    .contains("timed out"));
            }
            let shutdown = ShutdownSignal::new().unwrap();
            let mut state = INITIAL;
            match send_daemon_control_request(client, &request(), &shutdown, &mut state) {
                Err(DaemonRequestError::Failed(message)) => assert!(message.contains("timed out")),
                _ => panic!("silent peer did not time out"),
            }
            assert_eq!(state, INITIAL);
        });
    }

    #[test]
    fn shutdown_interrupts_a_pending_daemon_request() {
        with_connection(|_, client| {
            let shutdown = ShutdownSignal::new().unwrap();
            assert_eq!(
                unsafe { libc::pthread_kill(libc::pthread_self(), libc::SIGTERM) },
                0
            );
            let mut state = INITIAL;
            assert!(matches!(
                send_daemon_control_request(client, &request(), &shutdown, &mut state),
                Err(DaemonRequestError::Shutdown)
            ));
            assert!(!shutdown.consume().unwrap());
            assert_eq!(state, INITIAL);
        });
    }
}
