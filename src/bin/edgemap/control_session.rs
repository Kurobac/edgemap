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
            control::ServerPacket::Error { code, message } => {
                return Err(format!("{code}: {message}"));
            }
            packet => return Err(format!("unexpected control response: {packet:?}")),
        }
    }
}
