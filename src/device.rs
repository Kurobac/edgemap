use std::fs;
use std::io;
use std::io::Write;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use log::{debug, info, warn};

const HIDRAW_DEV_DIR: &str = "/dev";
const HIDRAW_PREFIX: &str = "hidraw";

pub const SONY_VID: u16 = 0x054C;
pub const DS5_PID: u16 = 0x0CE6;
pub const DS5_EDGE_PID: u16 = 0x0DF2;

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub path: PathBuf,
    pub vid: u16,
    pub pid: u16,
    pub is_edge: bool,
}

impl DeviceInfo {
    pub fn device_name(&self) -> &str {
        if self.is_edge {
            "DualSense Edge"
        } else {
            "DualSense"
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct HidrawDevinfo {
    bustype: u32,
    vendor: u16,
    product: u16,
}

fn hidraw_get_raw_info(fd: RawFd, info: &mut HidrawDevinfo) -> io::Result<()> {
    let request = ioc_read(0x03, std::mem::size_of::<HidrawDevinfo>());
    let ret = unsafe {
        libc::ioctl(fd, request, info as *mut HidrawDevinfo)
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

const IOC_READ: u64 = 2;
const IOC_WRITE: u64 = 1;
const IOC_READWRITE: u64 = IOC_READ | IOC_WRITE;

fn ioc_read(nr: u32, size: usize) -> u64 {
    (IOC_READ << 30) | ((b'H' as u64) << 8) | ((nr as u64) << 0) | ((size as u64) << 16)
}

fn ioc_readwrite(nr: u32, size: usize) -> u64 {
    (IOC_READWRITE << 30) | ((b'H' as u64) << 8) | ((nr as u64) << 0) | ((size as u64) << 16)
}

pub fn hidraw_get_report_descriptor(fd: RawFd) -> io::Result<Vec<u8>> {
    let mut desc_size: i32 = 0;
    let request = ioc_read(0x01, std::mem::size_of::<i32>());
    let ret = unsafe {
        libc::ioctl(fd, request, &mut desc_size as *mut i32)
    };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    if desc_size <= 0 || desc_size > 4096 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("invalid descriptor size: {desc_size}")));
    }

    let request = ioc_read(0x02, 4100);
    let mut buf = [0u8; 4100];
    // Write expected length for kernel >= 6.7 (reads len from caller);
    // harmless on older kernels (they write the whole struct, overwriting this).
    let len = (desc_size as u32).min(4095);
    buf[0..4].copy_from_slice(&len.to_ne_bytes());

    let ret = unsafe {
        libc::ioctl(fd, request, buf.as_mut_ptr())
    };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(buf[4..4 + desc_size as usize].to_vec())
}

pub fn ioctl_get_feature_report(fd: RawFd, buf: &mut [u8]) -> io::Result<()> {
    let request = ioc_readwrite(0x07, buf.len());
    let ret = unsafe { libc::ioctl(fd, request, buf.as_mut_ptr()) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub struct HidrawDevice {
    fd: OwnedFd,

    sysfs_path: Option<PathBuf>,
    restored_paths: Vec<(PathBuf, u32, String)>,
    report_desc: Vec<u8>,
}

impl HidrawDevice {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)?;

        let fd = OwnedFd::from(file);
        let raw_fd = fd.as_raw_fd();
        let mut devinfo = HidrawDevinfo::default();
        hidraw_get_raw_info(raw_fd, &mut devinfo)
            .map_err(|e| io::Error::new(e.kind(), format!("HIDIOCGRAWINFO failed: {e}")))?;

        let sysfs_path = find_sysfs_hidraw(path);

        let report_desc = hidraw_get_report_descriptor(raw_fd)
            .unwrap_or_else(|e| {
                warn!("Failed to read HID descriptor from device ({e}), using built-in fallback");
                crate::descriptor::DS_EDGE_USB_DESCRIPTOR.to_vec()
            });

        let device = Self {
            fd,
            sysfs_path,
            restored_paths: Vec::new(),
            report_desc,
        };

        // validate device state: read first input report
        let mut buf = [0u8; 64];
        match device.read_input(&mut buf) {
            Ok(64) if buf[0] == 0x01 => {
                debug!("device state OK: first input report valid");
            }
            Ok(n) => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("unexpected first report: {n} bytes"),
                ));
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                debug!("device state OK: no data yet (non-blocking read)");
            }
            Err(e) => {
                return Err(io::Error::new(
                    e.kind(),
                    format!("device not responding: {e}"),
                ));
            }
        }

        Ok(device)
    }

    pub fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    pub fn report_descriptor(&self) -> &[u8] {
        &self.report_desc
    }

    pub fn read_input(&self, buf: &mut [u8]) -> io::Result<usize> {
        let n = unsafe {
            libc::read(
                self.fd.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            )
        };
        if n < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }

    pub fn write_output(&self, data: &[u8]) -> io::Result<usize> {
        let n = unsafe {
            libc::write(
                self.fd.as_raw_fd(),
                data.as_ptr() as *const libc::c_void,
                data.len(),
            )
        };
        if n < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }

    pub fn send_feature_report(&self, buf: &[u8]) -> io::Result<()> {
        let request = ioc_readwrite(0x06, buf.len());
        let ret = unsafe { libc::ioctl(self.fd.as_raw_fd(), request, buf.as_ptr()) };
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

fn restrict_node(path: &Path, restored: &mut Vec<(PathBuf, u32, String)>) -> io::Result<()> {
        let orig = fs::metadata(path)?.permissions().mode();

        let acl_data = std::process::Command::new("getfacl")
            .args(["--absolute-names", &path.to_string_lossy()])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();

        let _ = std::process::Command::new("setfacl")
            .args(["-b", &path.to_string_lossy()])
            .output();

        fs::set_permissions(path, std::fs::Permissions::from_mode(0o000))?;
        restored.push((path.to_path_buf(), orig, acl_data));
        Ok(())
    }

    pub fn restrict_evdev_nodes(&mut self) -> io::Result<()> {
        let mut hidden: Vec<String> = Vec::new();

        if let Some(ref sysfs) = self.sysfs_path {
            let input_dir = sysfs.join("device/input");
            if !input_dir.exists() {
                debug!("No input directory at {:?}", input_dir);
                return Ok(());
            }

            if let Ok(entries) = fs::read_dir(&input_dir) {
                for input_entry in entries.flatten() {
                    let input_path = input_entry.path();
                    if !input_path.is_dir() || !input_path.file_name()
                        .map_or(false, |n| n.to_string_lossy().starts_with("input"))
                    {
                        continue;
                    }

                    if let Ok(ev_entries) = fs::read_dir(&input_path) {
                        for ev_entry in ev_entries.flatten() {
                            let ev_path = ev_entry.path();
                            let ev_name = ev_path.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("");
                            if ev_name.starts_with("event") || ev_name.starts_with("js") {
                                let dev_path = PathBuf::from("/dev/input").join(ev_name);
                                if dev_path.exists() {
                                    Self::restrict_node(&dev_path, &mut self.restored_paths)?;
                                    hidden.push(ev_name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(ref sysfs) = self.sysfs_path {
            let devname = sysfs.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let hidraw_path = PathBuf::from("/dev").join(devname);
            if hidraw_path.exists() {
                Self::restrict_node(&hidraw_path, &mut self.restored_paths)?;
                hidden.push(devname.to_string());
            }
        }

        info!("hidden {} physical device nodes", hidden.len());
        for name in &hidden {
            debug!("  restricted {name}");
        }

        Ok(())
    }

    pub fn clear_restored_paths(&mut self) {
        self.restored_paths.clear();
    }

    pub fn re_restrict_self(&mut self) {
        if let Some(ref sysfs) = self.sysfs_path {
            let devname = match sysfs.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => return,
            };
            let hidraw_path = PathBuf::from("/dev").join(&devname);
            if hidraw_path.exists() {
                if let Ok(meta) = fs::metadata(&hidraw_path) {
                    if meta.permissions().mode() & 0o777 != 0 {
                        info!("re-restricting hidraw node after udev reset");
                        Self::restrict_node(&hidraw_path, &mut self.restored_paths).ok();
                    }
                }
            }
        }
    }

    fn restore_permissions(&self) {
        if self.restored_paths.is_empty() {
            return;
        }
        info!("restore {} device nodes", self.restored_paths.len());
        let mut acl_batch = String::new();
        for (path, orig_mode, acl_data) in &self.restored_paths {
            if !path.exists() {
                continue;
            }
            if let Err(e) = fs::set_permissions(path, std::fs::Permissions::from_mode(*orig_mode))
            {
                log::warn!("Failed to restore permissions on {:?}: {e}", path);
            } else {
                log::debug!("Restored permissions on {:?}", path);
            }

            if !acl_data.is_empty() {
                acl_batch.push_str(acl_data);
            }
        }

        if !acl_batch.is_empty() {
            let mut child = match std::process::Command::new("setfacl")
                .arg("--restore=-")
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("Failed to spawn setfacl: {e}");
                    return;
                }
            };
            if let Err(e) = child.stdin.take().unwrap().write_all(acl_batch.as_bytes()) {
                log::warn!("Failed to write ACL data to setfacl: {e}");
            }
            let _ = child.wait();
        }
    }
}

impl Drop for HidrawDevice {
    fn drop(&mut self) {
        self.restore_permissions();
    }
}

fn is_physical_device(hidraw_name: &str) -> bool {
    if let Ok(uevent) = fs::read_to_string(format!("/sys/class/hidraw/{hidraw_name}/device/uevent")) {
        if uevent.contains("DRIVER=uhid") {
            debug!("skipping virtual UHID device {hidraw_name}");
            return false;
        }
    }
    true
}

pub fn find_dualsense() -> Option<DeviceInfo> {
    let dir = match fs::read_dir(HIDRAW_DEV_DIR) {
        Ok(d) => d,
        Err(_) => return None,
    };

    let mut found: Option<DeviceInfo> = None;

    for entry in dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with(HIDRAW_PREFIX) {
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

        if devinfo.vendor != SONY_VID {
            continue;
        }
        if devinfo.product != DS5_EDGE_PID && devinfo.product != DS5_PID {
            continue;
        }

        if !is_physical_device(&name_str) {
            continue;
        }

        if let Some(ref existing) = found {
            warn!(
                "multiple DualSense devices found (at {} and {}); using the first, additional devices are not supported",
                existing.path.display(),
                raw_path.display()
            );
            continue;
        }

        found = Some(DeviceInfo {
            path: raw_path.clone(),
            vid: devinfo.vendor,
            pid: devinfo.product,
            is_edge: devinfo.product == DS5_EDGE_PID,
        });
    }

    found
}

fn find_sysfs_hidraw(hidraw_path: &Path) -> Option<PathBuf> {
    let devname = hidraw_path.file_name()?.to_str()?;
    let sysfs_path = PathBuf::from(format!("/sys/class/hidraw/{devname}"));

    if sysfs_path.exists() {
        Some(sysfs_path)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_sysfs_hidraw() {
        let path = Path::new("/dev/hidraw0");
        let _sysfs = find_sysfs_hidraw(path);
    }
}
