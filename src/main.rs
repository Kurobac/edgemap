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
) -> bool {
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
                Ok(0) => return false,
                Ok(_) => (
                    fds[0].revents().unwrap_or(PollFlags::empty()),
                    fds[1].revents().unwrap_or(PollFlags::empty()),
                ),
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => {
                    error!("Failed while waiting for retry delay: {e}");
                    return true;
                }
            }
        };
        let failure = PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL;
        if shutdown_events.intersects(failure) || control_events.intersects(failure) {
            error!("poll failure while waiting for retry delay");
            return true;
        }
        if shutdown_events.contains(PollFlags::POLLIN) {
            return shutdown.consume().unwrap_or(true);
        }
        if control_events.contains(PollFlags::POLLIN) {
            if let Err(e) = reject_inactive_control(control) {
                error!("Failed to handle control request while waiting: {e}");
                return true;
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
                    eprintln!("error: --config-path requires a path argument");
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

fn print_usage() {
    eprintln!(
        "dseuhid {} — DualSense UHID Proxy",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!();
    eprintln!("Usage: dseuhid [OPTIONS] [COMMAND]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  version         Print version and exit");
    eprintln!("  help            Print this help");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -c, --config-path <path>  Config file (resets to passthrough on reconnect)");
    eprintln!();
    eprintln!("Without a command, starts the UHID proxy daemon (requires root).");
}

use device::{
    find_dualsense, probe_dualsense, HidrawMonitor, HidrawWait, InputNodesWait,
};
use proxy::Proxy;
use shutdown::ShutdownSignal;
use uhid::UhidDevice;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() >= 2 {
        // reject duplicate subcommands: all known subcommands take no extra args
        let sub = args[1].as_str();
let known = matches!(sub, "version" | "--version" | "-V" | "help" | "--help" | "-h");
        if known && args.len() > 2 {
            eprintln!("error: '{}' takes no arguments", args[1]);
            eprintln!("Run 'dseuhid help' for usage.");
            std::process::exit(1);
        }
        match sub {
            "version" | "--version" | "-V" => {
                println!("dseuhid {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "help" | "--help" | "-h" => {
                print_usage();
                return;
            }
            _ => {
                if !sub.starts_with('-') {
                    eprintln!("error: unknown command '{}'", args[1]);
                    eprintln!("Run 'dseuhid help' for usage.");
                    std::process::exit(1);
                }
            }
        }
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if unsafe { libc::getuid() } != 0 {
        error!("dseuhid daemon requires root (needs /dev/uhid and /dev/hidraw)");
        std::process::exit(1);
    }

    let mut config_path = parse_config_path();
    if let Some(path) = config_path.as_deref() {
        let cfg = config::Config::load(path).unwrap_or_else(|e| {
            error!("Failed to load startup config: {e}");
            std::process::exit(1);
        });
        config::validate(&cfg).unwrap_or_else(|e| {
            error!("Startup config validation failed: {e}");
            std::process::exit(1);
        });
    }

    info!("DualSense UHID proxy starting");
    let shutdown = ShutdownSignal::new().unwrap_or_else(|e| {
        error!("Failed to initialize signal handling: {e}");
        std::process::exit(1);
    });

    let _daemon_lock = control::DaemonLock::acquire(std::path::Path::new(RUNTIME_DIR))
        .unwrap_or_else(|e| {
            error!("Failed to acquire daemon lock: {e}");
            std::process::exit(1);
        });
    let mut control = control::ControlServer::bind(
        std::path::Path::new(RUNTIME_DIR),
        control::ControlState {
            uhid_ready: false,
            needs_config: config_path.is_none(),
        },
    )
    .unwrap_or_else(|e| {
        error!("Failed to initialize control socket: {e}");
        std::process::exit(1);
    });

    'outer: loop {
        let dev_info = {
            let hidraw_monitor = match HidrawMonitor::new() {
                Ok(monitor) => monitor,
                Err(e) => {
                    error!("Failed to monitor hidraw/input devices through udev: {e}");
                    break 'outer;
                }
            };
            let mut found = match find_dualsense() {
                Ok(device) => device,
                Err(e) => {
                    error!("Failed to enumerate hidraw devices through udev: {e}");
                    break 'outer;
                }
            };
            if found.is_none() {
                info!("Waiting for DualSense device...");
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
                                    error!("Failed to enumerate hidraw devices through udev: {e}");
                                    break 'outer;
                                }
                            };
                            if found.is_some() {
                                continue;
                            }
                            info!("Waiting for DualSense device...");
                        }
                        Ok(InputNodesWait::Control) => {
                            if let Err(e) = reject_inactive_control(&mut control) {
                                error!("Failed to handle inactive control request: {e}");
                                break 'outer;
                            }
                            found = Some(device);
                        }
                        Ok(InputNodesWait::Shutdown) => break 'outer,
                        Err(e) => {
                            error!("Failed while waiting for input nodes to initialize: {e}");
                            break 'outer;
                        }
                    }
                }

                let paths = match hidraw_monitor.wait(&shutdown, control.as_fd()) {
                    Ok(HidrawWait::Devices(paths)) => paths,
                    Ok(HidrawWait::Control) => {
                        if let Err(e) = reject_inactive_control(&mut control) {
                            error!("Failed to handle inactive control request: {e}");
                            break 'outer;
                        }
                        continue;
                    }
                    Ok(HidrawWait::Shutdown) => break 'outer,
                    Err(e) => {
                        error!("Failed to wait for hidraw device events: {e}");
                        break 'outer;
                    }
                };
                for path in paths {
                    match probe_dualsense(&path) {
                        Ok(Some(device)) if found.is_none() => found = Some(device),
                        Ok(Some(device)) => warn!(
                            "multiple DualSense devices found (at {} and {}); using the first, additional devices are not supported",
                            found.as_ref().unwrap().path.display(),
                            device.path.display()
                        ),
                        Ok(None) => {}
                        Err(e)
                            if matches!(
                                e.raw_os_error(),
                                Some(libc::ENOENT | libc::ENODEV | libc::ENXIO)
                            ) => {}
                        Err(e) => {
                            warn!("Failed to probe new hidraw device {}: {e}", path.display())
                        }
                    }
                }
            }
        };

        info!(
            "found {} via {} ({:04x}:{:04x}) at {}",
            dev_info.device_name(),
            dev_info.transport.name(),
            dev_info.vid,
            dev_info.pid,
            dev_info.path.display()
        );

        let loaded_config = match config_path.as_deref() {
            Some(path) => {
                info!("Loading config from {path}");
                match config::Config::load(path) {
                    Ok(cfg) => match config::validate(&cfg) {
                        Ok(()) => Some(cfg),
                        Err(e) => {
                            error!("Config validation failed: {e}");
                            break 'outer;
                        }
                    },
                    Err(e) => {
                        error!("Failed to load config: {e}");
                        break 'outer;
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
                            warn!("{name}: not configured, passthrough");
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
                    error!("Failed to build mapping: {e}");
                    break 'outer;
                }
            },
            None => {
                info!("No config specified, running in passthrough mode");
                mapping::MappingConfig::default()
            }
        };
        let mapping = Arc::new(RwLock::new(mapping));

        let mut hidraw = match device::HidrawDevice::open(&dev_info.path) {
            Ok(d) => d,
            Err(e) => {
                error!("Failed to open hidraw device: {e}");
                if shutdown_during_retry_delay(&shutdown, &mut control) {
                    break 'outer;
                }
                continue;
            }
        };

        let mut uhid = match UhidDevice::open() {
            Ok(d) => d,
            Err(e) => {
                error!("Failed to open /dev/uhid: {e}");
                error!("Make sure the uhid kernel module is loaded (modprobe uhid)");
                if shutdown_during_retry_delay(&shutdown, &mut control) {
                    break 'outer;
                }
                continue;
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
                        debug!("GET_REPORT cache: read report 0x{:02x} from physical device", request.report_id);
                        report_cache.insert(request.report_id, data);
                    }
                    Err(_) => {
                        warn!("GET_REPORT cache: invalid report 0x{:02x}, using built-in fallback", request.report_id);
                    }
                }
            } else {
                warn!("GET_REPORT cache: failed to read 0x{:02x}, using built-in fallback", request.report_id);
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
            error!("Failed to create UHID device: {e}");
            continue;
        }

        info!("Created virtual HID device: {} (output: {})", target_identity.name, target_identity.label);

        if let Err(e) = hidraw.restrict_evdev_nodes() {
            warn!("Failed to restrict physical evdev nodes: {e}");
        }

        info!("Proxy starting");

        let config_path_str = config_path.as_deref().unwrap_or("");

        let keyboard = match keyboard::KeyboardDevice::open() {
            Ok(k) => {
                info!("uinput keyboard device created");
                k
            }
            Err(e) => {
                warn!("uinput not available ({e}), keyboard targets will be ignored");
                keyboard::KeyboardDevice::dummy()
            }
        };

        if let Err(e) = reject_inactive_control(&mut control) {
            error!("Failed to handle inactive control request: {e}");
            break 'outer;
        }
        let mut proxy = Proxy::new(hidraw, uhid, mapping, config_path_str, report_cache.into_inner(), codec_pipeline, dev_info.kind, output_device, keyboard);
        match proxy.run(&shutdown, &mut control) {
            proxy::ExitReason::ConfigChanged => {
                config_path = Some(proxy.config_path().to_string());
                let mut state = control.state();
                state.uhid_ready = false;
                state.needs_config = false;
                control.set_state(state);
                info!("output_device changed in config, recreating virtual device...");
            }
            proxy::ExitReason::DeviceGone => {
                config_path = None;
                proxy.forget_restore_on_physical_disconnect();
                drop(proxy);
                control.set_state(control::ControlState {
                    uhid_ready: false,
                    needs_config: true,
                });
                info!("Device disconnected, waiting for reconnect...");
                if shutdown_during_retry_delay(&shutdown, &mut control) {
                    break 'outer;
                }
            }
            proxy::ExitReason::UserShutdown => {
                info!("Shutting down.");
                // hidraw + uhid auto-dropped — permissions restored, UHID destroyed
                break 'outer;
            }
            proxy::ExitReason::FatalError => {
                error!("Fatal proxy error, shutting down.");
                // Exit through normal scope cleanup so permissions and UHID are restored.
                break 'outer;
            }
        }
    }

    drop(control);
    info!("Shutdown complete.");
}

#[cfg(test)]
mod main_tests {
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
                "/tmp/default.toml".to_string(),
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
