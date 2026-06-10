#[path = "../descriptor.rs"]
#[allow(dead_code)]
mod descriptor;
#[path = "../device.rs"]
#[allow(dead_code)]
mod device;
#[path = "../uhid.rs"]
mod uhid;

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::time::Duration;

use device::HidrawDevice;
use uhid::{UhidDevice, UhidEvent};

const SONY_VID: u16 = 0x054C;
const DS4_PID: u16 = 0x09CC;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct HidrawDevinfo {
    bustype: u32,
    vendor: u16,
    product: u16,
}

fn hidraw_get_raw_info(fd: RawFd, info: &mut HidrawDevinfo) -> io::Result<()> {
    let request = (2u64 << 30) | ((b'H' as u64) << 8) | 3 | ((std::mem::size_of::<HidrawDevinfo>() as u64) << 16);
    let ret = unsafe { libc::ioctl(fd, request, info as *mut HidrawDevinfo) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn is_physical_device(hidraw_name: &str) -> bool {
    if let Ok(uevent) = fs::read_to_string(format!("/sys/class/hidraw/{hidraw_name}/device/uevent")) {
        if uevent.contains("DRIVER=uhid") {
            return false;
        }
    }
    true
}

fn find_ds4_hidraw() -> Option<std::path::PathBuf> {
    let dir = fs::read_dir("/dev").ok()?;
    for entry in dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("hidraw") {
            continue;
        }
        let raw_path = entry.path();
        let file = match fs::OpenOptions::new().read(true).write(true).open(&raw_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let fd = file.as_raw_fd();
        let mut devinfo = HidrawDevinfo::default();
        if hidraw_get_raw_info(fd, &mut devinfo).is_err() {
            continue;
        }
        if devinfo.vendor != SONY_VID || devinfo.product != DS4_PID {
            continue;
        }
        if !is_physical_device(&name_str) {
            continue;
        }
        return Some(raw_path);
    }
    None
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let dev_path = loop {
        match find_ds4_hidraw() {
            Some(p) => break p,
            None => {
                log::info!("Waiting for DS4 (PID {:04x})...", DS4_PID);
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    };
    log::info!("Found DS4 at {}", dev_path.display());

    let mut hidraw = loop {
        match HidrawDevice::open(&dev_path) {
            Ok(d) => break d,
            Err(e) => {
                log::error!("Failed to open hidraw: {e}");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    };

    let mut uhid = loop {
        match UhidDevice::open() {
            Ok(d) => break d,
            Err(e) => {
                log::error!("Failed to open /dev/uhid: {e}");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    };

    let hidraw_fd = hidraw.as_raw_fd();
    let uhid_fd = uhid.as_raw_fd();

    // Pre-cache core feature reports from physical DS4
    let mut report_cache: HashMap<u8, Vec<u8>> = HashMap::new();
    for rnum in [0x02u8, 0xA3u8] {
        let mut buf = vec![rnum]; buf.resize(64, 0);
        if device::ioctl_get_feature_report(hidraw_fd, &mut buf).is_ok() {
            log::debug!("cached feature report 0x{rnum:02x}");
            report_cache.insert(rnum, buf);
        }
    }
    // Fake MAC — real MAC would collide with physical DS4 (#63)
    report_cache.insert(0x12, vec![0x12, 0xC0, 0x13, 0x37, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

    // Start epoll BEFORE UHID create — register both fds so probe events are caught
    let epfd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
    if epfd < 0 {
        log::error!("epoll_create1 failed: {}", io::Error::last_os_error());
        return;
    }

    let mut epfiles: [libc::epoll_event; 2] = unsafe { std::mem::zeroed() };
    epfiles[0].events = (libc::EPOLLIN as u32) | (libc::EPOLLERR as u32);
    epfiles[0].u64 = 0;
    unsafe { libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, hidraw_fd, &mut epfiles[0]); }

    epfiles[1].events = (libc::EPOLLIN as u32) | (libc::EPOLLERR as u32);
    epfiles[1].u64 = 1;
    unsafe { libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, uhid_fd, &mut epfiles[1]); }

    let mut events: [libc::epoll_event; 4] = unsafe { std::mem::zeroed() };
    let mut buf = [0u8; 64];

    if let Err(e) = uhid.create(
        "Wireless Controller",
        "",
        "",
        0x0003,
        SONY_VID as u32,
        DS4_PID as u32,
        0x0100,
        0,
        &descriptor::DS4_USB_DESCRIPTOR,
    ) {
        log::error!("Failed to create UHID device: {e}");
        return;
    }
    log::info!("Created virtual DS4 UHID device");

    if let Err(e) = hidraw.restrict_evdev_nodes() {
        log::info!("Failed to restrict physical evdev: {e}");
    }

    let ready = true;
    log::info!("DS4 passthrough running. Press Ctrl+C to stop.");

    loop {
        let nfds = match unsafe { libc::epoll_wait(epfd, events.as_mut_ptr(), 4, -1) } {
            -1 => {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                log::error!("epoll_wait: {e}");
                break;
            }
            n => n,
        };

        for i in 0..nfds as usize {
            if events[i].u64 == 0 {
                match hidraw.read_input(&mut buf) {
                    Ok(n) if n >= 64 => {
                        if ready {
                            buf[7] = buf[7].wrapping_add(1);
                            if let Err(e) = uhid.send_input(&buf) {
                                log::error!("UHID send_input: {e}");
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(ref e) if e.raw_os_error() == Some(libc::EIO) => {
                        log::error!("hidraw disconnected");
                        return;
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => log::error!("hidraw read: {e}"),
                }
            } else if events[i].u64 == 1 {
                match uhid.recv_event() {
                    Ok(Some(UhidEvent::Start)) => log::info!("UHID started"),
                    Ok(Some(UhidEvent::Stop)) => log::warn!("UHID stopped"),
                    Ok(Some(UhidEvent::Open)) => log::debug!("UHID opened"),
                    Ok(Some(UhidEvent::Close)) => log::debug!("UHID closed"),
                    Ok(Some(UhidEvent::Output { rtype, ref data })) if rtype == 1 => {
                        if let Err(e) = hidraw.write_output(data) {
                            log::warn!("output forward: {e}");
                        }
                    }
                    Ok(Some(UhidEvent::GetReport { id, rnum, rtype: _ })) => {
                        let reply = if let Some(data) = report_cache.get(&rnum) {
                            data.clone()
                        } else {
                            let mut buf = vec![rnum]; buf.resize(64, 0);
                            buf
                        };
                        if let Err(e) = uhid.send_get_report_reply(id, 0, &reply) {
                            log::warn!("GET_REPORT reply rnum={rnum}: {e}");
                        }
                    }
                    Ok(Some(UhidEvent::SetReport { id, rnum, rtype, ref data })) => {
                        if rtype == 0 {
                            let mut full = vec![rnum];
                            full.extend_from_slice(data);
                            let _ = hidraw.send_feature_report(&full);
                        }
                        if let Err(e) = uhid.send_set_report_reply(id, 0) {
                            log::warn!("SET_REPORT reply: {e}");
                        }
                    }
                    Ok(Some(ev)) => log::debug!("UHID: {ev:?}"),
                    Ok(None) => {}
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => log::error!("UHID recv: {e}"),
                }
            }
        }
    }

    log::info!("DS4 passthrough stopped.");
}
