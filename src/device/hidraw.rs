use std::fs;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use log::debug;

use super::discovery::{find_sysfs_hidraw, BUS_USB};
use super::permissions::NodePermissions;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct HidrawDevinfo {
    pub(super) bustype: u32,
    pub(super) vendor: u16,
    pub(super) product: u16,
}

pub(super) fn hidraw_get_raw_info(fd: RawFd, info: &mut HidrawDevinfo) -> io::Result<()> {
    let request = ioc_read(0x03, std::mem::size_of::<HidrawDevinfo>());
    let ret = unsafe { libc::ioctl(fd, request, info as *mut HidrawDevinfo) };
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
    (IOC_READ << 30) | ((b'H' as u64) << 8) | (nr as u64) | ((size as u64) << 16)
}

fn ioc_readwrite(nr: u32, size: usize) -> u64 {
    (IOC_READWRITE << 30) | ((b'H' as u64) << 8) | (nr as u64) | ((size as u64) << 16)
}

pub fn hidraw_get_report_descriptor(fd: RawFd) -> io::Result<Vec<u8>> {
    let mut desc_size: i32 = 0;
    let request = ioc_read(0x01, std::mem::size_of::<i32>());
    let ret = unsafe { libc::ioctl(fd, request, &mut desc_size as *mut i32) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    if desc_size <= 0 || desc_size > 4096 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid descriptor size: {desc_size}"),
        ));
    }

    let request = ioc_read(0x02, 4100);
    let mut buf = [0u8; 4100];
    let len = (desc_size as u32).min(4096);
    buf[0..4].copy_from_slice(&len.to_ne_bytes());

    let ret = unsafe { libc::ioctl(fd, request, buf.as_mut_ptr()) };
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
    report_desc: Vec<u8>,
    permissions: NodePermissions,
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
            .map_err(|e| io::Error::new(e.kind(), format!("failed to read HID descriptor: {e}")))?;

        let device = Self {
            fd,
            report_desc,
            permissions: NodePermissions::new(sysfs_path),
        };

        if devinfo.bustype == BUS_USB {
            let mut buf = [0u8; 64];
            match device.read_input(&mut buf) {
                Ok(64) if buf[0] == 0x01 => {
                    debug!("device check passed: first USB input report valid");
                }
                Ok(n) => {
                    return Err(io::Error::other(format!(
                        "unexpected first USB report: {n} bytes"
                    )));
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    debug!("device check deferred: no USB input available");
                }
                Err(e) => {
                    return Err(io::Error::new(
                        e.kind(),
                        format!("USB device not responding: {e}"),
                    ));
                }
            }
        } else {
            debug!("first input report check skipped for non-USB source");
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
        } else if n as usize != data.len() {
            Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!(
                    "short hidraw output write: wrote {n} of {} bytes",
                    data.len()
                ),
            ))
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

    pub fn restrict_evdev_nodes(&mut self) -> io::Result<()> {
        self.permissions.restrict()
    }

    pub fn clear_restored_paths(&mut self) {
        self.permissions.forget();
    }

    pub fn re_restrict_self(&mut self) {
        self.permissions.re_restrict();
    }
}
