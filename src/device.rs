use std::fs;
use std::io;
use std::io::Write;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use log::{debug, info, warn};
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use udev::{Device as UdevDevice, Enumerator, EventType, MonitorBuilder, MonitorSocket};

use crate::shutdown::{unblock_shutdown_signals_in_child, ShutdownSignal};

const HIDRAW_PREFIX: &str = "hidraw";

fn is_hidraw_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(HIDRAW_PREFIX))
}

pub struct HidrawMonitor {
    socket: MonitorSocket,
}

pub enum HidrawWait {
    Devices(Vec<PathBuf>),
    Control,
    Shutdown,
}

pub enum InputNodesWait {
    Ready,
    Removed,
    Control,
    Shutdown,
}

enum MonitorActivity {
    Device,
    Control,
    Shutdown,
}

enum InputNodesState {
    Ready,
    Pending,
    Removed,
}

impl HidrawMonitor {
    pub fn new() -> io::Result<Self> {
        let socket = MonitorBuilder::new()?
            .match_subsystem("hidraw")?
            .match_subsystem("input")?
            .listen()?;
        Ok(Self { socket })
    }

    pub fn wait(
        &self,
        shutdown: &ShutdownSignal,
        control: BorrowedFd<'_>,
    ) -> io::Result<HidrawWait> {
        match self.wait_for_event(shutdown, control)? {
            MonitorActivity::Shutdown => return Ok(HidrawWait::Shutdown),
            MonitorActivity::Control => return Ok(HidrawWait::Control),
            MonitorActivity::Device => {}
        }
        Ok(HidrawWait::Devices(self.read_paths()))
    }

    pub fn wait_for_input_nodes(
        &self,
        hidraw_path: &Path,
        shutdown: &ShutdownSignal,
        control: BorrowedFd<'_>,
    ) -> io::Result<InputNodesWait> {
        loop {
            match input_nodes_state(hidraw_path)? {
                InputNodesState::Ready => return Ok(InputNodesWait::Ready),
                InputNodesState::Removed => return Ok(InputNodesWait::Removed),
                InputNodesState::Pending => {}
            }

            match self.wait_for_event(shutdown, control)? {
                MonitorActivity::Shutdown => return Ok(InputNodesWait::Shutdown),
                MonitorActivity::Control => return Ok(InputNodesWait::Control),
                MonitorActivity::Device => {}
            }
            self.socket.iter().for_each(drop);
        }
    }

    fn wait_for_event(
        &self,
        shutdown: &ShutdownSignal,
        control: BorrowedFd<'_>,
    ) -> io::Result<MonitorActivity> {
        loop {
            let monitor_fd = unsafe { BorrowedFd::borrow_raw(self.socket.as_raw_fd()) };
            let mut fds = [
                PollFd::new(monitor_fd, PollFlags::POLLIN),
                PollFd::new(shutdown.as_fd(), PollFlags::POLLIN),
                PollFd::new(control, PollFlags::POLLIN),
            ];
            match poll(&mut fds, PollTimeout::NONE) {
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(error) => return Err(error.into()),
            }
            let monitor_events = fds[0].revents().unwrap_or(PollFlags::empty());
            let shutdown_events = fds[1].revents().unwrap_or(PollFlags::empty());
            let control_events = fds[2].revents().unwrap_or(PollFlags::empty());
            let failure = PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL;
            if monitor_events.intersects(failure) {
                return Err(io::Error::other("udev device monitor poll failure"));
            }
            if shutdown_events.intersects(failure) {
                return Err(io::Error::other("shutdown signalfd poll failure"));
            }
            if control_events.intersects(failure) {
                return Err(io::Error::other("control epoll poll failure"));
            }
            if shutdown_events.contains(PollFlags::POLLIN) {
                shutdown.consume()?;
                return Ok(MonitorActivity::Shutdown);
            }
            if control_events.contains(PollFlags::POLLIN) {
                return Ok(MonitorActivity::Control);
            }
            if monitor_events.contains(PollFlags::POLLIN) {
                return Ok(MonitorActivity::Device);
            }
        }
    }

    fn read_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for event in self.socket.iter() {
            if event.event_type() != EventType::Add || !event.is_initialized() {
                continue;
            }
            if let Some(path) = event.devnode().filter(|path| is_hidraw_path(path)) {
                paths.push(path.to_path_buf());
            }
        }
        paths
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
        let mut command = std::process::Command::new("getfacl");
        command.args(["--absolute-names", &path.to_string_lossy()]);
        unblock_shutdown_signals_in_child(&mut command);
        let output = command.output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "getfacl failed with {}",
                output.status
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn clear_acl(path: &Path) {
        let mut command = std::process::Command::new("setfacl");
        command.args(["-b", &path.to_string_lossy()]);
        unblock_shutdown_signals_in_child(&mut command);
        match command.output() {
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
        let mut failures: Vec<String> = Vec::new();

        if let Some(ref sysfs) = self.sysfs_path {
            let devname = sysfs.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let hidraw_path = PathBuf::from("/dev").join(devname);
            if hidraw_path.exists() {
                match Self::restrict_node(&hidraw_path, &mut self.restored_nodes) {
                    Ok(()) => hidden.push(devname.to_string()),
                    Err(error) => failures.push(format!(
                        "failed to restrict {}: {error}",
                        hidraw_path.display()
                    )),
                }
            }

            match associated_input_nodes(sysfs) {
                Ok(nodes) => {
                    for node in nodes {
                        if let Some(dev_path) = node.dev_path.filter(|path| path.exists()) {
                            match Self::restrict_node(&dev_path, &mut self.restored_nodes) {
                                Ok(()) => hidden.push(node.name),
                                Err(error) => failures.push(format!(
                                    "failed to restrict {}: {error}",
                                    dev_path.display()
                                )),
                            }
                        }
                    }
                }
                Err(error) => failures.push(format!(
                    "failed to enumerate associated input nodes: {error}"
                )),
            }
        }

        info!("hidden {} physical device nodes", hidden.len());
        for name in &hidden {
            debug!("  restricted {name}");
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(failures.join("; ")))
        }
    }

    pub fn clear_restored_paths(&mut self) {
        self.restored_nodes.clear();
    }

    pub fn re_restrict_self(&mut self) {
        let mut restricted = Vec::new();
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
                Ok(()) => restricted.push(node.path.clone()),
                Err(e) => warn!("Failed to re-restrict {:?}: {e}", node.path),
            }
        }

        if !restricted.is_empty() {
            info!(
                "re-restricted {} device nodes after external permission reset",
                restricted.len()
            );
            for path in restricted {
                debug!("  re-restricted {}", path.display());
            }
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
            let mut command = std::process::Command::new("setfacl");
            command
                .arg("-P")
                .arg("--restore=-")
                .stdin(std::process::Stdio::piped());
            unblock_shutdown_signals_in_child(&mut command);
            let child = command.spawn();
            match child {
                Ok(mut child) => {
                    if let Some(mut stdin) = child.stdin.take() {
                        if let Err(e) = stdin.write_all(acl_batch.as_bytes()) {
                            log::warn!("Failed to write ACL data to setfacl: {e}");
                        }
                    }
                    match child.wait() {
                        Ok(status) if status.success() => {}
                        Ok(status) => log::warn!("setfacl restore failed with {status}"),
                        Err(e) => log::warn!("Failed to wait for setfacl restore: {e}"),
                    }
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

pub fn find_dualsense() -> io::Result<Option<DeviceInfo>> {
    let mut enumerator = Enumerator::new()?;
    enumerator.match_subsystem("hidraw")?;
    enumerator.match_is_initialized()?;

    let mut found: Option<DeviceInfo> = None;

    for device in enumerator.scan_devices()? {
        let Some(raw_path) = device.devnode() else {
            continue;
        };
        let info = match probe_dualsense(raw_path) {
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

    Ok(found)
}

pub fn probe_dualsense(path: &Path) -> io::Result<Option<DeviceInfo>> {
    if !is_hidraw_path(path) {
        return Ok(None);
    }
    let name = path.file_name().unwrap().to_str().unwrap();

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

struct AssociatedInputNode {
    name: String,
    dev_path: Option<PathBuf>,
    initialized: bool,
}

impl AssociatedInputNode {
    fn is_ready(&self) -> bool {
        self.initialized
            && self
                .dev_path
                .as_ref()
                .is_some_and(|path| path.exists())
    }
}

fn associated_input_nodes(hidraw_sysfs: &Path) -> io::Result<Vec<AssociatedInputNode>> {
    let hid_parent = fs::canonicalize(hidraw_sysfs.join("device"))?;
    let parent = UdevDevice::from_syspath(&hid_parent)?;
    let mut enumerator = Enumerator::new()?;
    enumerator.match_parent(&parent)?;
    enumerator.match_subsystem("input")?;

    let mut nodes = Vec::new();
    for device in enumerator.scan_devices()? {
        let Some(name) = device.sysname().to_str() else {
            continue;
        };
        if !name.starts_with("event") && !name.starts_with("js") {
            continue;
        }
        nodes.push(AssociatedInputNode {
            name: name.to_string(),
            dev_path: device.devnode().map(Path::to_path_buf),
            initialized: device.is_initialized(),
        });
    }
    Ok(nodes)
}

fn input_nodes_state(hidraw_path: &Path) -> io::Result<InputNodesState> {
    if !hidraw_path.exists() {
        return Ok(InputNodesState::Removed);
    }
    let Some(sysfs) = find_sysfs_hidraw(hidraw_path) else {
        return Ok(InputNodesState::Removed);
    };
    let nodes = match associated_input_nodes(&sysfs) {
        Ok(nodes) => nodes,
        Err(_) if !hidraw_path.exists() => return Ok(InputNodesState::Removed),
        Err(error) => return Err(error),
    };
    if nodes.is_empty() {
        debug!(
            "waiting for associated input nodes to appear for {}",
            hidraw_path.display()
        );
        return Ok(InputNodesState::Pending);
    }
    let pending: Vec<_> = nodes
        .iter()
        .filter(|node| !node.is_ready())
        .map(|node| node.name.as_str())
        .collect();
    if !pending.is_empty() {
        debug!("waiting for udev to initialize input nodes: {}", pending.join(", "));
        return Ok(InputNodesState::Pending);
    }
    Ok(InputNodesState::Ready)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_sysfs_hidraw() {
        let path = Path::new("/dev/hidraw0");
        let _sysfs = find_sysfs_hidraw(path);
        assert!(is_hidraw_path(Path::new("/dev/hidraw12")));
        assert!(!is_hidraw_path(Path::new("/dev/uhid")));
    }

    #[test]
    fn probe_ignores_non_hidraw_paths_without_opening_them() {
        assert!(probe_dualsense(Path::new("/dev/not-a-hidraw-node"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn associated_input_node_requires_initialized_existing_devnode() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "dseuhid-input-node-{}-{unique}",
            std::process::id(),
        ));
        let mut node = AssociatedInputNode {
            name: "event-test".to_string(),
            dev_path: None,
            initialized: true,
        };
        assert!(!node.is_ready());

        node.dev_path = Some(path.clone());
        assert!(!node.is_ready());
        std::fs::write(&path, []).unwrap();
        assert!(node.is_ready());

        node.initialized = false;
        assert!(!node.is_ready());
        std::fs::remove_file(path).unwrap();
    }
}
