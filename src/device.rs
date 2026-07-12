use std::fs;
use std::io;
use std::io::Write;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use log::{debug, info, warn};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, WatchDescriptor};

const HIDRAW_DEV_DIR: &str = "/dev";
const HIDRAW_PREFIX: &str = "hidraw";

fn is_hidraw_name(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| name.starts_with(HIDRAW_PREFIX))
}

pub struct HidrawMonitor {
    inotify: Inotify,
    watch: WatchDescriptor,
}

impl HidrawMonitor {
    pub fn new() -> io::Result<Self> {
        let inotify = Inotify::init(InitFlags::IN_CLOEXEC)?;
        let watch = inotify
            .add_watch(
                HIDRAW_DEV_DIR,
                AddWatchFlags::IN_CREATE | AddWatchFlags::IN_MOVED_TO,
            )?;
        Ok(Self { inotify, watch })
    }

    pub fn wait(&self) -> io::Result<Vec<PathBuf>> {
        let events = self.inotify.read_events()?;
        let mut paths = Vec::new();
        for event in events {
            if event.mask.contains(AddWatchFlags::IN_Q_OVERFLOW) {
                return Err(io::Error::other("hidraw inotify event queue overflowed"));
            }
            if event.wd != self.watch {
                continue;
            }
            if let Some(name) = event.name.filter(|name| is_hidraw_name(name)) {
                paths.push(Path::new(HIDRAW_DEV_DIR).join(name));
            }
        }
        Ok(paths)
    }
}

pub const SONY_VID: u16 = 0x054C;
pub const DS5_PID: u16 = 0x0CE6;
pub const DS5_EDGE_PID: u16 = 0x0DF2;
pub const DS4_PID: u16 = 0x09CC;

const BUS_USB: u32 = 0x0003;
const BUS_BLUETOOTH: u32 = 0x0005;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SonyDeviceKind {
    DualSense,
    DualSenseEdge,
}

impl SonyDeviceKind {
    pub fn from_pid(pid: u16) -> Option<Self> {
        match pid {
            DS5_PID => Some(Self::DualSense),
            DS5_EDGE_PID => Some(Self::DualSenseEdge),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::DualSense => "DualSense",
            Self::DualSenseEdge => "DualSense Edge",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceTransport {
    Usb,
    Bluetooth,
}

impl SourceTransport {
    fn from_bustype(bustype: u32) -> Option<Self> {
        match bustype {
            BUS_USB => Some(Self::Usb),
            BUS_BLUETOOTH => Some(Self::Bluetooth),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Usb => "USB",
            Self::Bluetooth => "Bluetooth",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub path: PathBuf,
    pub vid: u16,
    pub pid: u16,
    pub kind: SonyDeviceKind,
    pub transport: SourceTransport,
}

impl DeviceInfo {
    pub fn device_name(&self) -> &str {
        self.kind.name()
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
    (IOC_READ << 30) | ((b'H' as u64) << 8) | (nr as u64) | ((size as u64) << 16)
}

fn ioc_readwrite(nr: u32, size: usize) -> u64 {
    (IOC_READWRITE << 30) | ((b'H' as u64) << 8) | (nr as u64) | ((size as u64) << 16)
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
    let len = (desc_size as u32).min(4096);
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
    restored_nodes: Vec<RestoredNode>,
    report_desc: Vec<u8>,
}

struct RestoredNode {
    path: PathBuf,
    mode: u32,
    acl: String,
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
            sysfs_path,
            restored_nodes: Vec::new(),
            report_desc,
        };

        if devinfo.bustype == BUS_USB {
            // Validate USB device state by reading the first full input report.
            // Bluetooth uses a different report envelope and is handled below.
            let mut buf = [0u8; 64];
            match device.read_input(&mut buf) {
                Ok(64) if buf[0] == 0x01 => {
                    debug!("device state OK: first USB input report valid");
                }
                Ok(n) => {
                    return Err(io::Error::other(
                        format!("unexpected first USB report: {n} bytes"),
                    ));
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    debug!("device state OK: no USB data yet (non-blocking read)");
                }
                Err(e) => {
                    return Err(io::Error::new(
                        e.kind(),
                        format!("USB device not responding: {e}"),
                    ));
                }
            }
        } else {
            // DualSense Bluetooth may deliver a minimal 0x01 report before the
            // full 0x31 state report. Runtime input decoding already validates
            // the 0x31 report shape and CRC, so avoid failing open on a valid
            // but unusable-for-us BT packet here.
            debug!("skipping first input report validation for non-USB device");
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
                format!("short hidraw output write: wrote {n} of {} bytes", data.len()),
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

    fn read_acl(path: &Path) -> io::Result<String> {
        let output = std::process::Command::new("getfacl")
            .args(["--absolute-names", &path.to_string_lossy()])
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "getfacl failed with {}",
                output.status
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn clear_acl(path: &Path) {
        match std::process::Command::new("setfacl")
            .args(["-b", &path.to_string_lossy()])
            .output()
        {
            Ok(output) if output.status.success() => {}
            Ok(output) => warn!("Failed to clear ACL on {:?}: {}", path, output.status),
            Err(e) => warn!("Failed to clear ACL on {:?}: {e}", path),
        }
    }

    fn restrict_node(path: &Path, restored: &mut Vec<RestoredNode>) -> io::Result<()> {
        let mode = fs::metadata(path)?.permissions().mode();
        let acl = Self::read_acl(path).unwrap_or_else(|e| {
            warn!("Failed to save ACL on {:?}: {e}", path);
            String::new()
        });

        restored.push(RestoredNode {
            path: path.to_path_buf(),
            mode,
            acl,
        });

        Self::clear_acl(path);
        fs::set_permissions(path, std::fs::Permissions::from_mode(0o000))?;
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

            match fs::read_dir(&input_dir) {
                Ok(entries) => for input_entry in entries.flatten() {
                    let input_path = input_entry.path();
                    if !input_path.is_dir()
                        || !input_path
                            .file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with("input"))
                    {
                        continue;
                    }

                    match fs::read_dir(&input_path) {
                        Ok(ev_entries) => for ev_entry in ev_entries.flatten() {
                            let ev_path = ev_entry.path();
                            let ev_name =
                                ev_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                            if ev_name.starts_with("event") || ev_name.starts_with("js") {
                                let dev_path = PathBuf::from("/dev/input").join(ev_name);
                                if dev_path.exists() {
                                    Self::restrict_node(&dev_path, &mut self.restored_nodes)?;
                                    hidden.push(ev_name.to_string());
                                }
                            }
                        },
                        Err(e) => warn!("Failed to read input child directory {:?}: {e}", input_path),
                    }
                },
                Err(e) => warn!("Failed to read input directory {:?}: {e}", input_dir),
            }
        }

        if let Some(ref sysfs) = self.sysfs_path {
            let devname = sysfs.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let hidraw_path = PathBuf::from("/dev").join(devname);
            if hidraw_path.exists() {
                Self::restrict_node(&hidraw_path, &mut self.restored_nodes)?;
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
        self.restored_nodes.clear();
    }

    pub fn re_restrict_self(&mut self) {
        let mut restricted = 0;
        for node in &mut self.restored_nodes {
            let mode = match fs::metadata(&node.path) {
                Ok(metadata) => metadata.permissions().mode(),
                Err(e) => {
                    warn!("Failed to inspect {:?} for re-restriction: {e}", node.path);
                    continue;
                }
            };
            if mode & 0o777 == 0 {
                continue;
            }

            match Self::read_acl(&node.path) {
                Ok(acl) => {
                    node.mode = mode;
                    node.acl = acl;
                }
                Err(e) => {
                    warn!(
                        "Failed to refresh ACL snapshot on {:?}, keeping previous snapshot: {e}",
                        node.path
                    );
                }
            }

            Self::clear_acl(&node.path);
            match fs::set_permissions(&node.path, fs::Permissions::from_mode(0o000)) {
                Ok(()) => restricted += 1,
                Err(e) => warn!("Failed to re-restrict {:?}: {e}", node.path),
            }
        }

        if restricted > 0 {
            info!("re-restricted {restricted} device nodes after external permission reset");
        }
    }

    fn restore_permissions(&self) {
        if self.restored_nodes.is_empty() {
            return;
        }
        info!("restore {} device nodes", self.restored_nodes.len());
        let mut acl_batch = String::new();
        for node in &self.restored_nodes {
            if !node.path.exists() {
                continue;
            }
            if let Err(e) =
                fs::set_permissions(&node.path, std::fs::Permissions::from_mode(node.mode))
            {
                log::warn!("Failed to restore permissions on {:?}: {e}", node.path);
            } else {
                log::debug!("Restored permissions on {:?}", node.path);
            }

            if !node.acl.is_empty() {
                acl_batch.push_str(&node.acl);
            }
        }

        if !acl_batch.is_empty() {
            let child = std::process::Command::new("setfacl")
                .arg("--restore=-")
                .stdin(std::process::Stdio::piped())
                .spawn();
            match child {
                Ok(mut child) => {
                    if let Some(mut stdin) = child.stdin.take() {
                        if let Err(e) = stdin.write_all(acl_batch.as_bytes()) {
                            log::warn!("Failed to write ACL data to setfacl: {e}");
                        }
                    }
                    let _ = child.wait();
                }
                Err(e) => log::warn!("Failed to spawn setfacl: {e}"),
            }
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
        let raw_path = entry.path();
        let info = match probe_dualsense(&raw_path) {
            Ok(Some(info)) => info,
            Ok(None) | Err(_) => continue,
        };

        if let Some(ref existing) = found {
            warn!(
                "multiple DualSense devices found (at {} and {}); using the first, additional devices are not supported",
                existing.path.display(),
                info.path.display()
            );
            continue;
        }

        found = Some(info);
    }

    found
}

pub fn probe_dualsense(path: &Path) -> io::Result<Option<DeviceInfo>> {
    let name = match path.file_name().and_then(|name| name.to_str()) {
        Some(name) if name.starts_with(HIDRAW_PREFIX) => name,
        _ => return Ok(None),
    };

    let file = fs::OpenOptions::new().read(true).write(true).open(path)?;
    let mut devinfo = HidrawDevinfo::default();
    hidraw_get_raw_info(file.as_raw_fd(), &mut devinfo)?;

    if devinfo.vendor != SONY_VID {
        return Ok(None);
    }
    let kind = match SonyDeviceKind::from_pid(devinfo.product) {
        Some(kind) => kind,
        None => return Ok(None),
    };
    let transport = match SourceTransport::from_bustype(devinfo.bustype) {
        Some(transport) => transport,
        None => {
            debug!(
                "skipping unsupported DualSense {name} (bustype={})",
                devinfo.bustype
            );
            return Ok(None);
        }
    };
    if !is_physical_device(name) {
        return Ok(None);
    }

    Ok(Some(DeviceInfo {
        path: path.to_path_buf(),
        vid: devinfo.vendor,
        pid: devinfo.product,
        kind,
        transport,
    }))
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
        assert!(is_hidraw_name(std::ffi::OsStr::new("hidraw12")));
        assert!(!is_hidraw_name(std::ffi::OsStr::new("uhid")));
    }

    #[test]
    fn probe_ignores_non_hidraw_paths_without_opening_them() {
        assert!(probe_dualsense(Path::new("/dev/not-a-hidraw-node"))
            .unwrap()
            .is_none());
    }
}
