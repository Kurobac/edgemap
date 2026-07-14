use dseuhid::{config, control, shutdown};

use log::{error, info, warn};
use std::time::{Duration, Instant};

use nix::poll::{poll, PollFd, PollFlags, PollTimeout};

static RUNTIME_DIR: &str = "/run/dseuhid";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitOutcome {
    Elapsed,
    Shutdown,
    Fatal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DaemonExit {
    Shutdown,
    Fatal,
}

pub(crate) fn reject_inactive_control(control: &mut control::ControlServer) -> std::io::Result<()> {
    for pending in control.drain_requests()? {
        control.reply_error(pending.client, "not-ready", "UHID proxy is not ready");
    }
    Ok(())
}

fn shutdown_during_retry_delay(
    shutdown: &ShutdownSignal,
    control: &mut control::ControlServer,
) -> WaitOutcome {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout_ms = remaining.as_millis().min(u16::MAX as u128) as u16;
        let (shutdown_events, control_events) = {
            let mut fds = [
                PollFd::new(shutdown.as_fd(), PollFlags::POLLIN),
                PollFd::new(control.as_fd(), PollFlags::POLLIN),
            ];
            match poll(&mut fds, PollTimeout::from(timeout_ms)) {
                Ok(0) => return WaitOutcome::Elapsed,
                Ok(_) => (
                    fds[0].revents().unwrap_or(PollFlags::empty()),
                    fds[1].revents().unwrap_or(PollFlags::empty()),
                ),
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => {
                    error!("failed to poll during retry delay: {e}");
                    return WaitOutcome::Fatal;
                }
            }
        };
        let failure = PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL;
        if shutdown_events.intersects(failure) || control_events.intersects(failure) {
            error!("poll reported a failure during retry delay");
            return WaitOutcome::Fatal;
        }
        if shutdown_events.contains(PollFlags::POLLIN) {
            return match shutdown.consume() {
                Ok(true) => WaitOutcome::Shutdown,
                Ok(false) => {
                    error!("shutdown signal fd was readable but contained no signal");
                    WaitOutcome::Fatal
                }
                Err(e) => {
                    error!("failed to read shutdown signal: {e}");
                    WaitOutcome::Fatal
                }
            };
        }
        if control_events.contains(PollFlags::POLLIN) {
            if let Err(e) = reject_inactive_control(control) {
                error!("failed to handle control request during retry delay: {e}");
                return WaitOutcome::Fatal;
            }
        }
    }
}

use crate::device::{find_dualsense, probe_dualsense, HidrawMonitor, HidrawWait, InputNodesWait};
use shutdown::ShutdownSignal;

pub(crate) fn run(config_path: Option<String>) -> DaemonExit {
    let mut active_config = config_path.map(|path| {
        let active = config::ActiveConfig::read(&path).unwrap_or_else(|e| {
            error!("failed to load startup config: {e}");
            std::process::exit(1);
        });
        let cfg = active.parse().unwrap_or_else(|e| {
            error!("failed to load startup config: {e}");
            std::process::exit(1);
        });
        config::validate(&cfg).unwrap_or_else(|e| {
            error!("startup config validation failed: {e}");
            std::process::exit(1);
        });
        active
    });

    info!("dseuhid daemon starting");
    let shutdown = ShutdownSignal::new().unwrap_or_else(|e| {
        error!("failed to initialize signal handling: {e}");
        std::process::exit(1);
    });

    let _daemon_lock = control::DaemonLock::acquire(std::path::Path::new(RUNTIME_DIR))
        .unwrap_or_else(|e| {
            error!("failed to acquire daemon lock: {e}");
            std::process::exit(1);
        });
    let mut control = control::ControlServer::bind(
        std::path::Path::new(RUNTIME_DIR),
        control::ControlState {
            uhid_ready: false,
            needs_config: active_config.is_none(),
        },
    )
    .unwrap_or_else(|e| {
        error!("failed to initialize control socket: {e}");
        std::process::exit(1);
    });

    let daemon_exit = 'outer: loop {
        let dev_info = {
            let hidraw_monitor = match HidrawMonitor::new() {
                Ok(monitor) => monitor,
                Err(e) => {
                    error!("failed to create udev monitor for hidraw/input devices: {e}");
                    break 'outer DaemonExit::Fatal;
                }
            };
            let mut found = match find_dualsense() {
                Ok(device) => device,
                Err(e) => {
                    error!("failed to enumerate hidraw devices through udev: {e}");
                    break 'outer DaemonExit::Fatal;
                }
            };
            if found.is_none() {
                info!("waiting for a controller");
            }

            'wait_for_device: loop {
                if let Some(device) = found.take() {
                    match hidraw_monitor.wait_for_input_nodes(
                        &device.path,
                        &shutdown,
                        control.as_fd(),
                    ) {
                        Ok(InputNodesWait::Ready) => break 'wait_for_device device,
                        Ok(InputNodesWait::Removed) => {
                            found = match find_dualsense() {
                                Ok(device) => device,
                                Err(e) => {
                                    error!("failed to enumerate hidraw devices through udev: {e}");
                                    break 'outer DaemonExit::Fatal;
                                }
                            };
                            if found.is_some() {
                                continue;
                            }
                            info!("waiting for a controller");
                        }
                        Ok(InputNodesWait::Control) => {
                            if let Err(e) = reject_inactive_control(&mut control) {
                                error!(
                                    "failed to reject control request while proxy is inactive: {e}"
                                );
                                break 'outer DaemonExit::Fatal;
                            }
                            found = Some(device);
                        }
                        Ok(InputNodesWait::Shutdown) => break 'outer DaemonExit::Shutdown,
                        Err(e) => {
                            error!("failed while waiting for associated input nodes: {e}");
                            break 'outer DaemonExit::Fatal;
                        }
                    }
                }

                let paths = match hidraw_monitor.wait(&shutdown, control.as_fd()) {
                    Ok(HidrawWait::Devices(paths)) => paths,
                    Ok(HidrawWait::Control) => {
                        if let Err(e) = reject_inactive_control(&mut control) {
                            error!("failed to reject control request while proxy is inactive: {e}");
                            break 'outer DaemonExit::Fatal;
                        }
                        continue;
                    }
                    Ok(HidrawWait::Shutdown) => break 'outer DaemonExit::Shutdown,
                    Err(e) => {
                        error!("failed to wait for hidraw device events: {e}");
                        break 'outer DaemonExit::Fatal;
                    }
                };
                for path in paths {
                    match probe_dualsense(&path) {
                        Ok(Some(device)) if found.is_none() => found = Some(device),
                        Ok(Some(device)) => {
                            warn!("multiple controllers found; using the first");
                            warn!(
                                "controller selection: selected={}, ignored={}",
                                found.as_ref().unwrap().path.display(),
                                device.path.display()
                            );
                        }
                        Ok(None) => {}
                        Err(e)
                            if matches!(
                                e.raw_os_error(),
                                Some(libc::ENOENT | libc::ENODEV | libc::ENXIO)
                            ) => {}
                        Err(e) => {
                            warn!(
                                "failed to probe hidraw device: path={}, error={e}",
                                path.display()
                            )
                        }
                    }
                }
            }
        };

        info!(
            "controller found: name={}, transport={}",
            dev_info.device_name(),
            dev_info.transport.name()
        );
        info!(
            "controller identity: id={:04x}:{:04x}, path={}",
            dev_info.vid,
            dev_info.pid,
            dev_info.path.display()
        );

        match crate::session::run(&dev_info, &active_config, &shutdown, &mut control) {
            crate::session::SessionExit::ConfigChanged(config) => {
                active_config = config;
                let mut state = control.state();
                state.uhid_ready = false;
                state.needs_config = false;
                control.set_state(state);
                info!("output device changed; recreating virtual HID device");
            }
            crate::session::SessionExit::DeviceGone { reset_config } => {
                if reset_config {
                    active_config = None;
                    control.set_state(control::ControlState {
                        uhid_ready: false,
                        needs_config: true,
                    });
                }
                info!("controller disconnected; waiting for reconnection");
                match shutdown_during_retry_delay(&shutdown, &mut control) {
                    WaitOutcome::Elapsed => {}
                    WaitOutcome::Shutdown => break 'outer DaemonExit::Shutdown,
                    WaitOutcome::Fatal => break 'outer DaemonExit::Fatal,
                }
            }
            crate::session::SessionExit::Shutdown => {
                info!("dseuhid daemon stopping");
                break 'outer DaemonExit::Shutdown;
            }
            crate::session::SessionExit::Fatal => {
                error!("fatal proxy error; dseuhid daemon stopping");
                break 'outer DaemonExit::Fatal;
            }
        }
    };

    drop(control);
    info!("dseuhid daemon stopped");
    daemon_exit
}

#[cfg(test)]
mod daemon_tests {
    use super::*;

    #[test]
    fn inactive_control_requests_are_rejected_without_queueing() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "dseuhid-inactive-control-{}-{unique}",
            std::process::id()
        ));
        let mut server = control::ControlServer::bind(
            &dir,
            control::ControlState {
                uhid_ready: false,
                needs_config: true,
            },
        )
        .unwrap();
        let client = control::ControlClient::connect(&dir.join(control::SOCKET_FILE_NAME)).unwrap();
        assert!(server.drain_requests().unwrap().is_empty());
        assert!(matches!(
            client.receive().unwrap(),
            Some(control::ServerPacket::Hello(_))
        ));

        client
            .send_request(&control::ControlRequest::SwitchConfig(
                config::ActiveConfig::from_content(
                    "/tmp/default.toml".to_string(),
                    "version = 2\n".to_string(),
                )
                .unwrap(),
            ))
            .unwrap();
        reject_inactive_control(&mut server).unwrap();
        assert_eq!(
            client.receive().unwrap(),
            Some(control::ServerPacket::Error {
                code: "not-ready".to_string(),
                message: "UHID proxy is not ready".to_string(),
            })
        );

        drop(client);
        drop(server);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
