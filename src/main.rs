mod config;
mod codec;
#[allow(dead_code)]
mod control;
mod descriptor;
mod device;
mod keyboard;
mod mapping;
mod proxy;
mod report;
mod shutdown;
mod uhid;

use log::{debug, error, info, warn};
use std::env;
use std::sync::{Arc, RwLock};
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
enum DaemonExit {
    Shutdown,
    Fatal,
}

fn is_controller_gone_error(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOENT | libc::EIO | libc::ENODEV | libc::ENXIO)
    )
}

fn reject_inactive_control(control: &mut control::ControlServer) -> std::io::Result<()> {
    for pending in control.drain_requests()? {
        control.reply_error(
            pending.client,
            "not-ready",
            "UHID proxy is not ready",
        );
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

fn parse_config_path() -> Option<String> {
    let args: Vec<String> = env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-c" | "--config-path" => {
                if i + 1 >= args.len() {
                    eprintln!("error: option '--config-path' requires a path");
                    std::process::exit(1);
                }
                return Some(args[i + 1].clone());
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn usage_text() -> String {
    format!(
        concat!(
            "dseuhid {} — DualSense UHID proxy\n",
            "\n",
            "Usage: dseuhid [OPTIONS] [COMMAND]\n",
            "\n",
            "Commands:\n",
            "  version                   Print version and exit\n",
            "  help                      Print help\n",
            "\n",
            "Options:\n",
            "  -c, --config-path <PATH>  Load a config file; reconnect resets to passthrough\n",
            "\n",
            "Without a command, start the UHID proxy daemon (requires root).\n",
        ),
        env!("CARGO_PKG_VERSION")
    )
}

fn print_usage(to_stdout: bool) {
    let usage = usage_text();
    if to_stdout {
        print!("{usage}");
    } else {
        eprint!("{usage}");
    }
}

use device::{
    find_dualsense, probe_dualsense, HidrawMonitor, HidrawWait, InputNodesWait,
};
use proxy::{Proxy, ProxyInit};
use shutdown::ShutdownSignal;
use uhid::UhidDevice;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() >= 2 {
        // reject duplicate subcommands: all known subcommands take no extra args
        let sub = args[1].as_str();
        let known = matches!(sub, "version" | "--version" | "-V" | "help" | "--help" | "-h");
        if known && args.len() > 2 {
            eprintln!("error: command '{}' does not accept arguments", args[1]);
            eprintln!("hint: run 'dseuhid help' for usage");
            std::process::exit(1);
        }
        match sub {
            "version" | "--version" | "-V" => {
                println!("dseuhid {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "help" | "--help" | "-h" => {
                print_usage(true);
                return;
            }
            _ => {
                if !sub.starts_with('-') {
                    eprintln!("error: unknown command '{}'", args[1]);
                    eprintln!("hint: run 'dseuhid help' for usage");
                    std::process::exit(1);
                }
            }
        }
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if unsafe { libc::getuid() } != 0 {
        error!("dseuhid daemon requires root");
        std::process::exit(1);
    }

    if let Err(e) = proxy::validate_repeat_env() {
        error!("{e}");
        std::process::exit(1);
    }

    let mut active_config = parse_config_path().map(|path| {
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
                                error!("failed to reject control request while proxy is inactive: {e}");
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
                            warn!("failed to probe hidraw device: path={}, error={e}", path.display())
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

        let loaded_config = match active_config.as_ref() {
            Some(active) => {
                info!("loading config: source={}", active.source());
                match active.parse() {
                    Ok(cfg) => Some(cfg),
                    Err(e) => {
                        error!("failed to parse active config: {e}");
                        break 'outer DaemonExit::Fatal;
                    }
                }
            }
            None => None,
        };
        let output_device = loaded_config
            .as_ref()
            .map_or_else(|| "auto".to_string(), |cfg| cfg.output_device.clone());
        let codec_pipeline = codec::CodecPipeline::from_device_and_output(
            dev_info.kind,
            dev_info.transport,
            &output_device,
        );
        let mapping = match loaded_config.as_ref() {
            Some(cfg) => match cfg.to_mapping_config() {
                Ok(mapping) => {
                    for name in config::ALL_BUTTON_NAMES {
                        if !cfg.buttons.contains_key(*name) {
                            debug!("button not configured; using passthrough: button={name}");
                        }
                    }
                    proxy::warn_ignored_edge_passthroughs(
                        cfg,
                        dev_info.kind,
                        codec_pipeline.target,
                    );
                    mapping
                }
                Err(e) => {
                    error!("failed to build mapping: {e}");
                    break 'outer DaemonExit::Fatal;
                }
            },
            None => {
                info!("no config specified; using passthrough mode");
                mapping::MappingConfig::default()
            }
        };
        let mapping = Arc::new(RwLock::new(mapping));

        let mut hidraw = match device::HidrawDevice::open(&dev_info.path) {
            Ok(d) => d,
            Err(e) if is_controller_gone_error(&e) => {
                warn!("failed to open controller: {e}");
                info!("controller disconnected; waiting for reconnection");
                match shutdown_during_retry_delay(&shutdown, &mut control) {
                    WaitOutcome::Elapsed => {}
                    WaitOutcome::Shutdown => break 'outer DaemonExit::Shutdown,
                    WaitOutcome::Fatal => break 'outer DaemonExit::Fatal,
                }
                continue;
            }
            Err(e) => {
                error!("failed to open hidraw device: {e}");
                break 'outer DaemonExit::Fatal;
            }
        };

        let mut uhid = match UhidDevice::open() {
            Ok(d) => d,
            Err(e) => {
                error!("failed to open /dev/uhid: {e}");
                error!("UHID kernel module may be unavailable; verify with: modprobe uhid");
                break 'outer DaemonExit::Fatal;
            }
        };

        let mut report_cache = codec::FeatureReportCache::new();
        // Physical transport decides which real-device reports are safe to read.
        // main owns the hidraw ioctl; codec owns the source/target policy.
        for request in codec_pipeline.physical.feature_reports_to_cache(codec_pipeline.target) {
            let mut buf = vec![request.report_id];
            buf.resize(request.size, 0);
            if device::ioctl_get_feature_report(hidraw.as_raw_fd(), &mut buf).is_ok() {
                match codec_pipeline.physical.decode_feature_report(*request, buf) {
                    Ok(data) => {
                        debug!("feature report cached: report_id=0x{:02x}", request.report_id);
                        report_cache.insert(request.report_id, data);
                    }
                    Err(_) => {
                        warn!("invalid feature report; using target response: report_id=0x{:02x}", request.report_id);
                    }
                }
            } else {
                warn!("failed to read feature report; using target response: report_id=0x{:02x}", request.report_id);
            }
        }
        codec_pipeline.target.seed_feature_reports(&mut report_cache);

        // Target devices are USB-only for now. Auto mode reuses the physical
        // USB descriptor; revisit this when a BT source can back a USB target.
        let target_identity = codec_pipeline.target.usb_identity(&dev_info, hidraw.report_descriptor());
        if let Err(e) = uhid.create(
            &target_identity.name,
            "",
            target_identity.uniq,
            0x0003, // BUS_USB
            dev_info.vid as u32,
            target_identity.product_id,
            0x0100,
            0,
            target_identity.report_descriptor,
        ) {
            error!("failed to create virtual HID device: {e}");
            break 'outer DaemonExit::Fatal;
        }

        info!("virtual HID device created: name={}, output={}", target_identity.name, target_identity.label);

        if let Err(e) = hidraw.restrict_evdev_nodes() {
            warn!("failed to restrict input nodes: {e}");
        }

        info!("proxy initializing");

        let keyboard = match keyboard::KeyboardDevice::open() {
            Ok(k) => {
                info!("virtual keyboard created");
                k
            }
            Err(e) => {
                warn!("virtual keyboard unavailable; keyboard targets disabled: {e}");
                keyboard::KeyboardDevice::dummy()
            }
        };

        if let Err(e) = reject_inactive_control(&mut control) {
            error!("failed to reject control request while proxy is inactive: {e}");
            break 'outer DaemonExit::Fatal;
        }
        let mut proxy = Proxy::new(ProxyInit {
            hidraw,
            uhid,
            keyboard,
            mapping,
            active_config: active_config.clone(),
            report_cache,
            codec: codec_pipeline,
            source_kind: dev_info.kind,
            output_device_config: output_device,
        });
        match proxy.run(&shutdown, &mut control) {
            proxy::ExitReason::ConfigChanged => {
                active_config = proxy.active_config().cloned();
                let mut state = control.state();
                state.uhid_ready = false;
                state.needs_config = false;
                control.set_state(state);
                info!("output device changed; recreating virtual HID device");
            }
            proxy::ExitReason::DeviceGone => {
                active_config = None;
                proxy.forget_restore_on_physical_disconnect();
                drop(proxy);
                control.set_state(control::ControlState {
                    uhid_ready: false,
                    needs_config: true,
                });
                info!("controller disconnected; waiting for reconnection");
                match shutdown_during_retry_delay(&shutdown, &mut control) {
                    WaitOutcome::Elapsed => {}
                    WaitOutcome::Shutdown => break 'outer DaemonExit::Shutdown,
                    WaitOutcome::Fatal => break 'outer DaemonExit::Fatal,
                }
            }
            proxy::ExitReason::UserShutdown => {
                info!("dseuhid daemon stopping");
                // hidraw + uhid auto-dropped — permissions restored, UHID destroyed
                break 'outer DaemonExit::Shutdown;
            }
            proxy::ExitReason::FatalError => {
                error!("fatal proxy error; dseuhid daemon stopping");
                // Exit through normal scope cleanup so permissions and UHID are restored.
                break 'outer DaemonExit::Fatal;
            }
        }
    };

    drop(control);
    info!("dseuhid daemon stopped");
    if daemon_exit == DaemonExit::Fatal {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod main_tests {
    use super::*;

    #[test]
    fn usage_uses_conventional_placeholders() {
        let usage = usage_text();
        assert!(usage.contains("Usage: dseuhid [OPTIONS] [COMMAND]"));
        assert!(usage.contains("--config-path <PATH>"));
        assert!(!usage.contains("<path>"));
    }

    #[test]
    fn controller_open_retries_only_device_gone_errors() {
        for errno in [libc::ENOENT, libc::EIO, libc::ENODEV, libc::ENXIO] {
            assert!(is_controller_gone_error(&std::io::Error::from_raw_os_error(errno)));
        }
        for errno in [libc::EACCES, libc::EINVAL, libc::EBADF] {
            assert!(!is_controller_gone_error(&std::io::Error::from_raw_os_error(errno)));
        }
    }

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
