mod config;
mod codec;
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
use std::os::fd::FromRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{Arc, RwLock};
use std::time::Duration;

static FIFO_DIR: &str = "/run/dseuhid";
static FIFO_PATH: &str = "/run/dseuhid/control";
static FIFO_TEMP_PATH: &str = "/run/dseuhid/.control.tmp";
static PID_PATH: &str = "/run/dseuhid/pid";
static CONNECTED_PATH: &str = "/run/dseuhid/connected";
static NEEDS_CONFIG_PATH: &str = "/run/dseuhid/needs-config";

fn write_runtime_state(path: &str, content: &[u8]) -> std::io::Result<()> {
    let path = std::path::Path::new(path);
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other(format!("invalid runtime state path: {path:?}")))?;
    let temp_path = path.with_file_name(format!(".{}.tmp", file_name.to_string_lossy()));

    std::fs::write(&temp_path, content)?;
    if let Err(error) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(temp_path);
        return Err(error);
    }
    Ok(())
}

pub(crate) fn write_connected_state(connected: bool) -> std::io::Result<()> {
    let content: &[u8] = if connected {
        b"connected\n"
    } else {
        b"disconnected\n"
    };
    write_runtime_state(CONNECTED_PATH, content)
}

pub(crate) fn write_needs_config(needs_config: bool) -> std::io::Result<()> {
    let content: &[u8] = if needs_config { b"true\n" } else { b"false\n" };
    write_runtime_state(NEEDS_CONFIG_PATH, content)
}

fn setup_fifo(needs_config: bool) -> std::fs::File {
    std::fs::create_dir_all(FIFO_DIR).unwrap_or_else(|e| {
        eprintln!("error: cannot create {}: {e}", FIFO_DIR);
        std::process::exit(1);
    });
    // Build the control endpoint under a hidden name. Publishing it last makes
    // the visible FIFO a readiness marker for the complete runtime state.
    let _ = std::fs::remove_file(FIFO_PATH);
    let _ = std::fs::remove_file(FIFO_TEMP_PATH);
    let c_path =
        std::ffi::CString::new(FIFO_TEMP_PATH).expect("FIFO_TEMP_PATH contains null byte");
    let r = unsafe { libc::mkfifo(c_path.as_ptr(), 0o666) };
    if r != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
        eprintln!(
            "error: cannot create FIFO at {FIFO_TEMP_PATH}: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
    if unsafe { libc::chmod(c_path.as_ptr(), 0o666) } != 0 {
        log::warn!("failed to chmod FIFO: {}", std::io::Error::last_os_error());
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(FIFO_TEMP_PATH)
        .unwrap_or_else(|e| {
            eprintln!("error: cannot open FIFO: {e}");
            std::process::exit(1);
        });
    write_connected_state(false).unwrap_or_else(|e| {
        eprintln!("error: cannot initialize connected state: {e}");
        std::process::exit(1);
    });
    write_needs_config(needs_config).unwrap_or_else(|e| {
        eprintln!("error: cannot initialize needs-config state: {e}");
        std::process::exit(1);
    });
    if let Err(e) = std::fs::write(PID_PATH, std::process::id().to_string()) {
        log::warn!("failed to write PID file: {e}");
    }
    std::fs::rename(FIFO_TEMP_PATH, FIFO_PATH).unwrap_or_else(|e| {
        eprintln!("error: cannot publish FIFO at {FIFO_PATH}: {e}");
        std::process::exit(1);
    });
    file
}

fn teardown_fifo() {
    let _ = std::fs::remove_file(FIFO_PATH);
    let _ = std::fs::remove_file(FIFO_TEMP_PATH);
    let _ = std::fs::remove_file(PID_PATH);
    let _ = std::fs::remove_file(CONNECTED_PATH);
    let _ = std::fs::remove_file(NEEDS_CONFIG_PATH);
}

fn dup_fifo_fd(fifo_fd: &std::fs::File) -> std::fs::File {
    use std::os::fd::AsRawFd;
    let raw = fifo_fd.as_raw_fd();
    let fd = unsafe { libc::dup(raw) };
    if fd < 0 {
        error!("Failed to dup FIFO fd: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    unsafe { std::fs::File::from_raw_fd(fd) }
}

fn shutdown_during_retry_delay(shutdown: &ShutdownSignal) -> bool {
    match shutdown.wait_timeout(Duration::from_secs(2)) {
        Ok(requested) => requested,
        Err(e) => {
            error!("Failed while waiting for retry delay: {e}");
            true
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

    info!("DualSense UHID proxy starting");
    let shutdown = ShutdownSignal::new().unwrap_or_else(|e| {
        error!("Failed to initialize signal handling: {e}");
        std::process::exit(1);
    });

    // check for existing instance
    if let Ok(pid_str) = std::fs::read_to_string(PID_PATH) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            if unsafe { libc::kill(pid, 0) } == 0 {
                error!("another dseuhid instance is running (PID {pid})");
                std::process::exit(1);
            }
        }
    }

    let fifo_fd = setup_fifo(config_path.is_none());

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
                    match hidraw_monitor.wait_for_input_nodes(&device.path, &shutdown) {
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
                        Ok(InputNodesWait::Shutdown) => break 'outer,
                        Err(e) => {
                            error!("Failed while waiting for input nodes to initialize: {e}");
                            break 'outer;
                        }
                    }
                }

                let paths = match hidraw_monitor.wait(&shutdown) {
                    Ok(HidrawWait::Devices(paths)) => paths,
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

        let mut hidraw = match device::HidrawDevice::open(&dev_info.path) {
            Ok(d) => d,
            Err(e) => {
                error!("Failed to open hidraw device: {e}");
                if shutdown_during_retry_delay(&shutdown) {
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
                if shutdown_during_retry_delay(&shutdown) {
                    break 'outer;
                }
                continue;
            }
        };

        let output_device = config_path.as_ref()
            .and_then(|p| config::Config::load(p).ok())
            .map(|c| c.output_device)
            .unwrap_or_else(|| "auto".to_string());

        let codec_pipeline = codec::CodecPipeline::from_device_and_output(
            dev_info.kind,
            dev_info.transport,
            &output_device,
        );

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

        if let Err(e) = write_connected_state(true) {
            log::warn!("failed to write connected file: {e}");
        }

        if let Err(e) = hidraw.restrict_evdev_nodes() {
            warn!("Failed to restrict physical evdev nodes: {e}");
        }

        info!("Proxy starting");

        let mapping = match &config_path {
            Some(path) => {
                info!("Loading config from {path}");
                match config::Config::load(path) {
                    Ok(cfg) => match config::validate(&cfg) {
                        Err(e) => {
                            error!("Config validation failed: {e}");
                            std::process::exit(1);
                        }
                        Ok(()) => match cfg.to_mapping_config() {
                            Ok(m) => {
                                for name in config::ALL_BUTTON_NAMES {
                                    if !cfg.buttons.contains_key(*name) {
                                        warn!("{name}: not configured, passthrough");
                                    }
                                }
                                proxy::warn_ignored_edge_passthroughs(&cfg, dev_info.kind, codec_pipeline.target);
                                m
                            }
                            Err(e) => {
                                error!("Failed to build mapping: {e}");
                                std::process::exit(1);
                            }
                        },
                    },
                    Err(e) => {
                        error!("Failed to load config: {e}");
                        std::process::exit(1);
                    }
                }
            }
            None => {
                info!("No config specified, running in passthrough mode");
                mapping::MappingConfig::default()
            }
        };
        let mapping = Arc::new(RwLock::new(mapping));

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

        let mut proxy = Proxy::new(hidraw, uhid, mapping, config_path_str, report_cache.into_inner(), codec_pipeline, dev_info.kind, output_device, keyboard, dup_fifo_fd(&fifo_fd));
        match proxy.run(&shutdown) {
            proxy::ExitReason::ConfigChanged => {
                config_path = Some(proxy.config_path().to_string());
                info!("output_device changed in config, recreating virtual device...");
            }
            proxy::ExitReason::DeviceGone => {
                config_path = None;
                proxy.forget_restore_on_physical_disconnect();
                drop(proxy);
                if let Err(e) = write_needs_config(true) {
                    log::warn!("failed to write needs-config state: {e}");
                }
                info!("Device disconnected, waiting for reconnect...");
                if shutdown_during_retry_delay(&shutdown) {
                    break 'outer;
                }
            }
            proxy::ExitReason::UserShutdown => {
                info!("Shutting down.");
                // hidraw + uhid auto-dropped — permissions restored, UHID destroyed
                break 'outer;
            }
        }
    }

    teardown_fifo();
    info!("Shutdown complete.");
}
