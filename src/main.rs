mod descriptor;
mod device;
mod proxy;
mod report;
mod uhid;

use log::{error, info};
use std::process;

use device::find_dualsense;
use proxy::Proxy;
use uhid::UhidDevice;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let dev_info = match find_dualsense() {
        Some(d) => {
            info!(
                "Found {} ({:04x}:{:04x}) at {}",
                d.device_name(),
                d.vid,
                d.pid,
                d.path.display()
            );
            d
        }
        None => {
            error!("No DualSense controller found. Is it connected via USB?");
            process::exit(1);
        }
    };

    let mut hidraw = match device::HidrawDevice::open(&dev_info.path) {
        Ok(d) => d,
        Err(e) => {
            error!("Failed to open hidraw device: {e}");
            process::exit(1);
        }
    };

    let report_desc = {
        let desc = descriptor::dualsense_usb_descriptor();
        info!(
            "Using built-in DualSense HID descriptor ({} bytes)",
            desc.len()
        );
        desc
    };

    let mut uhid = match UhidDevice::open() {
        Ok(d) => d,
        Err(e) => {
            error!("Failed to open /dev/uhid: {e}");
            error!("Make sure the uhid kernel module is loaded (modprobe uhid)");
            error!("Also, /dev/uhid requires root access.");
            process::exit(1);
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
        error!("Descriptor was {} bytes", report_desc.len());
        process::exit(1);
    }

    info!("Created virtual HID device: {name}");

    if let Err(e) = hidraw.restrict_evdev_nodes() {
        info!("Failed to restrict physical evdev nodes: {e}");
        info!("You may see two controllers in games — select the virtual one.");
    } else {
        info!("Physical evdev nodes hidden — only virtual device is visible to games");
    }

    proxy::setup_signal_handler();

    let mut proxy = Proxy::new(hidraw, uhid);

    if let Err(e) = proxy.run() {
        error!("Proxy error: {e}");
    }

    info!("Shutdown complete.");
}
