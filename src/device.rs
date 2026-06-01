use std::fs;
use std::io;
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
    pub name: String,
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

fn hidraw_get_rd_size(fd: RawFd) -> io::Result<u32> {
    let mut size: u32 = 0;
    let request = ioc_read(0x05, std::mem::size_of::<u32>());
    let ret = unsafe {
        libc::ioctl(fd, request, &raw mut size)
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(size)
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

fn hid_descriptor_len(data: &[u8]) -> usize {
    let mut pos: usize = 0;
    let mut last_valid: usize = 0;

    while pos < data.len() {
        let prefix = data[pos];
        if prefix == 0 {
            break;
        }
        let mut size: usize = (prefix & 0x03) as usize;
        if size == 3 {
            size = 4;
        }

        pos += 1;
        if pos + size > data.len() {
            break;
        }

        last_valid = pos + size;
        pos += size;
    }

    last_valid
}

pub struct HidrawDevice {
    fd: OwnedFd,
    pub info: DeviceInfo,
    sysfs_path: Option<PathBuf>,
    restored_paths: Vec<(PathBuf, u32)>,
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

        let is_edge = devinfo.product == DS5_EDGE_PID;
        let sysfs_path = find_sysfs_hidraw(path);

        Ok(Self {
            fd,
            info: DeviceInfo {
                path: path.to_path_buf(),
                vid: devinfo.vendor,
                pid: devinfo.product,
                name: if is_edge {
                    "DualSense Edge".into()
                } else {
                    "DualSense".into()
                },
                is_edge,
            },
            sysfs_path,
            restored_paths: Vec::new(),
        })
    }

    pub fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
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

    pub fn get_report_descriptor(&self) -> io::Result<Vec<u8>> {
        if let Some(ref sysfs) = self.sysfs_path {
            let desc_path = sysfs.join("device/report_descriptor");
            if desc_path.exists() {
                let data = fs::read(&desc_path)?;
                if !data.is_empty() {
                    let actual_len = hid_descriptor_len(&data);
                    let mut desc = data;
                    desc.truncate(actual_len);
                    debug!(
                        "Read report descriptor from sysfs: {} bytes (parsed to {actual_len})",
                        desc.len()
                    );
                    return Ok(desc);
                }
            }
        }

        let raw_fd = self.fd.as_raw_fd();
        let rd_size = hidraw_get_rd_size(raw_fd)?;

        let mut desc = vec![0u8; rd_size as usize];
        let request = ioc_read(0x01, rd_size as usize);
        let ret = unsafe {
            libc::ioctl(raw_fd, request, desc.as_mut_ptr())
        };
        if ret < 0 {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!("HIDIOCGRDESC failed: {}", io::Error::last_os_error()),
            ))
        } else {
            Ok(desc)
        }
    }

    pub fn get_feature_report(&self, report_id: u8, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "buffer too small",
            ));
        }
        buf[0] = report_id;
        let request = ioc_readwrite(0x07, buf.len());
        let ret = unsafe { libc::ioctl(self.fd.as_raw_fd(), request, buf.as_mut_ptr()) };
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(ret as usize)
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

    fn restrict_node(path: &Path, restored: &mut Vec<(PathBuf, u32)>) -> io::Result<()> {
        let orig = fs::metadata(path)?.permissions().mode();
        fs::set_permissions(path, std::fs::Permissions::from_mode(0o000))?;
        restored.push((path.to_path_buf(), orig));
        Ok(())
    }

    pub fn restrict_evdev_nodes(&mut self) -> io::Result<()> {
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
                                    info!("Restricted {ev_name}");
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
                info!("Restricted {devname}");
            }
        }

        Ok(())
    }

    fn restore_permissions(&self) {
        for (path, orig_mode) in &self.restored_paths {
            if let Err(e) = fs::set_permissions(path, std::fs::Permissions::from_mode(*orig_mode))
            {
                log::warn!("Failed to restore permissions on {:?}: {e}", path);
            } else {
                log::info!("Restored permissions on {:?}", path);
            }
        }
    }
}

impl Drop for HidrawDevice {
    fn drop(&mut self) {
        self.restore_permissions();
    }
}

pub fn find_dualsense() -> Option<DeviceInfo> {
    let dir = match fs::read_dir(HIDRAW_DEV_DIR) {
        Ok(d) => d,
        Err(_) => return None,
    };

    for entry in dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with(HIDRAW_PREFIX) {
            continue;
        }

        let raw_path = entry.path();
        let path = raw_path.clone();

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
        if devinfo.product != DS5_PID && devinfo.product != DS5_EDGE_PID {
            continue;
        }

        let is_edge = devinfo.product == DS5_EDGE_PID;
        debug!(
            "Found {} at {}",
            if is_edge { "DualSense Edge" } else { "DualSense" },
            raw_path.display()
        );

        return Some(DeviceInfo {
            path,
            vid: devinfo.vendor,
            pid: devinfo.product,
            name: if is_edge {
                "DualSense Edge".into()
            } else {
                "DualSense".into()
            },
            is_edge,
        });
    }

    None
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
