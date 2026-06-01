mod config;
mod descriptor;
mod device;
mod mapping;
mod proxy;
mod report;
mod uhid;

use log::{error, info, warn};
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use device::find_dualsense;
use proxy::Proxy;
use uhid::UhidDevice;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("DualSense Edge UHID proxy starting");
    proxy::setup_signal_handler();
    proxy::setup_reload_handler();

    let report_desc = descriptor::dualsense_usb_descriptor();
    info!(
        "Using built-in DualSense HID descriptor ({} bytes)",
        report_desc.len()
    );

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

        let config_path = "/etc/dseuhid/config.toml";
        if !Path::new(config_path).exists() {
            if let Err(e) = std::fs::create_dir_all("/etc/dseuhid") {
                warn!("Cannot create /etc/dseuhid: {e}");
            }
            if let Err(e) = std::fs::write(config_path, config::default_content()) {
                warn!("Cannot create default config at {config_path}: {e}");
            } else {
                info!("Created default config at {config_path}");
            }
        }

        let mapping = Arc::new(RwLock::new(match config::Config::load(config_path) {
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

        let mut proxy = Proxy::new(hidraw, uhid, mapping, config_path);
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

    info!("Shutdown complete.");
}
