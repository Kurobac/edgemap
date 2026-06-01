mod descriptor;
mod device;
mod proxy;
mod report;
mod uhid;

use log::{error, info};
use std::time::Duration;

use device::find_dualsense;
use proxy::Proxy;
use uhid::UhidDevice;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("DualSense Edge UHID proxy starting");
    proxy::setup_signal_handler();

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
                        "Found {} ({:04x}:{:04x}) at {}",
                        d.device_name(),
                        d.vid,
                        d.pid,
                        d.path.display()
                    );
                    break d;
                }
                None => {
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
        } else {
            info!("Physical evdev nodes hidden — only virtual device is visible to games");
        }

        info!("Proxy starting");

        let mut proxy = Proxy::new(hidraw, uhid);
        match proxy.run() {
            proxy::ExitReason::DeviceGone => {
                proxy.skip_restore();
                info!("Device disconnected, waiting for reconnect...");
                // hidraw + uhid auto-dropped — permissions restored, UHID destroyed
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
