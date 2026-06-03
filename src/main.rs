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
use std::path::Path;
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
    let r = unsafe { libc::mkfifo(FIFO_PATH.as_ptr() as *const libc::c_char, 0o666) };
    if r != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
        eprintln!("error: cannot create FIFO at {FIFO_PATH}: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
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

fn parse_config_path() -> String {
    let args: Vec<String> = env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-c" | "--config-path" => {
                if i + 1 >= args.len() {
                    eprintln!("error: --config-path requires a path argument");
                    std::process::exit(1);
                }
                return args[i + 1].clone();
            }
            _ => {}
        }
        i += 1;
    }
    "/etc/dseuhid/config.toml".into()
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
    eprintln!("  -c, --config-path <path>  Config file (default: /etc/dseuhid/config.toml)");
    eprintln!();
    eprintln!("Without a command, starts the UHID proxy daemon (requires root).");
}

use device::find_dualsense;
use proxy::Proxy;
use uhid::UhidDevice;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() >= 2 {
        match args[1].as_str() {
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

        if let Err(e) = hidraw.restrict_evdev_nodes() {
            info!("Failed to restrict physical evdev nodes: {e}");
            info!("You may see two controllers in games — select the virtual one.");
        }

        info!("Proxy starting");

        let default_path = "/etc/dseuhid/config.toml";
        if config_path == default_path && !Path::new(&config_path).exists() {
            if let Err(e) = std::fs::create_dir_all("/etc/dseuhid") {
                warn!("Cannot create /etc/dseuhid: {e}");
            }
            if let Err(e) = std::fs::write(&config_path, config::default_content()) {
                warn!("Cannot create default config at {config_path}: {e}");
            } else {
                info!("Created default config at {config_path}");
            }
        }

        let mapping = Arc::new(RwLock::new(match config::Config::load(&config_path) {
            Ok(cfg) => {
                if let Err(e) = config::validate(&cfg) {
                    error!("Config validation failed: {e}");
                    error!("Running with no remapping.");
                    mapping::MappingConfig::default()
                } else {
                    match cfg.to_mapping_config() {
                        Ok(m) => {
                            info!("Loaded config from {config_path}");
                            // warn for missing button sections
                            for name in config::ALL_BUTTON_NAMES {
                                if !cfg.buttons.contains_key(*name) {
                                    warn!("{name}: not configured, passthrough");
                                }
                            }
                            m
                        }
                        Err(e) => {
                            error!("Failed to build mapping: {e}");
                            mapping::MappingConfig::default()
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to load config: {e}");
                mapping::MappingConfig::default()
            }
        }));

        let mut proxy = Proxy::new(hidraw, uhid, mapping, &config_path, dup_fifo_fd(&fifo_fd));
        match proxy.run() {
            proxy::ExitReason::DeviceGone => {
                proxy.skip_restore();
                info!("Device disconnected, waiting for reconnect...");
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
