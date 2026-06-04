mod config;
mod descriptor;
mod device;
mod mapping;
mod monitor;
mod proxy;
mod report;
mod touchdemo;
mod uhid;

use log::{error, info, warn};
use std::env;
use std::os::fd::FromRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{Arc, RwLock};
use std::time::Duration;

static FIFO_DIR: &str = "/run/dseuhid";
static FIFO_PATH: &str = "/run/dseuhid/control";
static PID_PATH: &str = "/run/dseuhid/pid";

fn setup_fifo() -> std::fs::File {
    std::fs::create_dir_all(FIFO_DIR).unwrap_or_else(|e| {
        eprintln!("error: cannot create {}: {e}", FIFO_DIR);
        std::process::exit(1);
    });
    // Remove stale FIFO from previous unclean exit, then create
    let _ = std::fs::remove_file(FIFO_PATH);
    let c_path = std::ffi::CString::new(FIFO_PATH).expect("FIFO_PATH contains null byte");
    let r = unsafe { libc::mkfifo(c_path.as_ptr(), 0o666) };
    if r != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
        eprintln!("error: cannot create FIFO at {FIFO_PATH}: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    unsafe { libc::chmod(c_path.as_ptr(), 0o666) };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(FIFO_PATH)
        .unwrap_or_else(|e| {
            eprintln!("error: cannot open FIFO: {e}");
            std::process::exit(1);
        });
    std::fs::write(PID_PATH, std::process::id().to_string()).ok();
    file
}

fn teardown_fifo() {
    let _ = std::fs::remove_file(FIFO_PATH);
    let _ = std::fs::remove_file(PID_PATH);
}

fn dup_fifo_fd(fifo_fd: &std::fs::File) -> std::fs::File {
    use std::os::fd::AsRawFd;
    let raw = fifo_fd.as_raw_fd();
    let fd = unsafe { libc::dup(raw) };
    if fd < 0 {
        error!("Failed to dup FIFO fd: {}", std::io::Error::last_os_error());
    }
    unsafe { std::fs::File::from_raw_fd(fd) }
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
        "dseuhid {} — DualSense Edge UHID Proxy",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!();
    eprintln!("Usage: dseuhid [OPTIONS] [COMMAND]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  monitor     Raw HID button debug tool");
    eprintln!("  touchdemo   Touchpad coordinate debug tool");
    eprintln!("  version     Print version and exit");
    eprintln!("  help        Print this help");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -c, --config-path <path>  Config file (passthrough if not set)");
    eprintln!();
    eprintln!("Without a command, starts the UHID proxy daemon (requires root).");
}

use device::find_dualsense;
use proxy::Proxy;
use uhid::UhidDevice;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() >= 2 {
        // reject duplicate subcommands: all known subcommands take no extra args
        let sub = args[1].as_str();
        let known = matches!(sub, "monitor" | "mon" | "touchdemo" | "touch" | "version" | "--version" | "-V" | "help" | "--help" | "-h");
        if known && args.len() > 2 {
            eprintln!("error: '{}' takes no arguments", args[1]);
            eprintln!("Run 'dseuhid help' for usage.");
            std::process::exit(1);
        }
        match sub {
            "monitor" | "mon" => {
                monitor::run();
                return;
            }
            "touchdemo" | "touch" => {
                touchdemo::run();
                return;
            }
            "version" | "--version" | "-V" => {
                println!("dseuhid {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "help" | "--help" | "-h" => {
                print_usage();
                return;
            }
            _ => {
                eprintln!("error: unknown command '{}'", args[1]);
                eprintln!("Run 'dseuhid help' for usage.");
                std::process::exit(1);
            }
        }
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config_path = parse_config_path();

    info!("DualSense Edge UHID proxy starting");
    proxy::setup_signal_handler();
    proxy::setup_reload_handler();

    let report_desc = descriptor::dualsense_usb_descriptor();
    info!(
        "Using built-in DualSense HID descriptor ({} bytes)",
        report_desc.len()
    );

    // check for existing instance
    if let Ok(pid_str) = std::fs::read_to_string(PID_PATH) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            if unsafe { libc::kill(pid, 0) } == 0 {
                error!("another dseuhid instance is running (PID {pid})");
                std::process::exit(1);
            }
        }
    }

    let fifo_fd = setup_fifo();

    'outer: loop {
        let dev_info = loop {
            if !proxy::is_running() {
                break 'outer;
            }
            match find_dualsense() {
                Some(d) => {
                    info!(
                        "found DualSense Edge ({:04x}:{:04x}) at {}",
                        d.vid,
                        d.pid,
                        d.path.display()
                    );
                    break d;
                }
                None => {
                    if proxy::try_clear_reload() {
                        info!("received reload signal (no device connected)");
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        };

        let mut hidraw = match device::HidrawDevice::open(&dev_info.path) {
            Ok(d) => d,
            Err(e) => {
                error!("Failed to open hidraw device: {e}");
                continue;
            }
        };

        let mut uhid = match UhidDevice::open() {
            Ok(d) => d,
            Err(e) => {
                error!("Failed to open /dev/uhid: {e}");
                error!("Make sure the uhid kernel module is loaded (modprobe uhid)");
                continue;
            }
        };

        let name = format!("{} Remapper", dev_info.device_name());
        if let Err(e) = uhid.create(
            &name,
            "",
            "",
            0x0003, // BUS_USB
            dev_info.vid as u32,
            dev_info.pid as u32,
            0x0100,
            0,
            &report_desc,
        ) {
            error!("Failed to create UHID device: {e}");
            continue;
        }

        info!("Created virtual HID device: {name}");

        let _ = std::fs::write("/run/dseuhid/connected", b"connected");

        if let Err(e) = hidraw.restrict_evdev_nodes() {
            info!("Failed to restrict physical evdev nodes: {e}");
            info!("You may see two controllers in games — select the virtual one.");
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
        let mut proxy = Proxy::new(hidraw, uhid, mapping, config_path_str, dup_fifo_fd(&fifo_fd));
        match proxy.run() {
            proxy::ExitReason::DeviceGone => {
                proxy.skip_restore();
                info!("Device disconnected, waiting for reconnect...");
                let _ = std::fs::write("/run/dseuhid/connected", b"disconnected");
                std::thread::sleep(Duration::from_secs(2));
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
