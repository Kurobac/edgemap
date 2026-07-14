use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use log::{debug, warn};
use udev::Enumerator;

use super::hidraw::{hidraw_get_raw_info, HidrawDevinfo};
#[cfg(test)]
use super::monitor::AssociatedInputNode;

const HIDRAW_PREFIX: &str = "hidraw";

pub(super) fn is_hidraw_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(HIDRAW_PREFIX))
}

pub const SONY_VID: u16 = 0x054C;
pub const DS5_PID: u16 = 0x0CE6;
pub const DS5_EDGE_PID: u16 = 0x0DF2;
pub const DS4_PID: u16 = 0x09CC;

pub(super) const BUS_USB: u32 = 0x0003;
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

fn is_physical_device(hidraw_name: &str) -> bool {
    if let Ok(uevent) = fs::read_to_string(format!("/sys/class/hidraw/{hidraw_name}/device/uevent"))
    {
        if is_uhid_uevent(&uevent) {
            debug!("virtual UHID device skipped: node={hidraw_name}");
            return false;
        }
    }
    true
}

fn is_uhid_uevent(uevent: &str) -> bool {
    uevent.lines().any(|line| line == "DRIVER=uhid")
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
            warn!("multiple controllers found; using the first");
            warn!(
                "controller selection: selected={}, ignored={}",
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
    if !is_physical_device(name) {
        return Ok(None);
    }

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
                "controller skipped due to unsupported bus type: node={name}, bustype={}",
                devinfo.bustype
            );
            return Ok(None);
        }
    };
    Ok(Some(DeviceInfo {
        path: path.to_path_buf(),
        vid: devinfo.vendor,
        pid: devinfo.product,
        kind,
        transport,
    }))
}

pub(super) fn find_sysfs_hidraw(hidraw_path: &Path) -> Option<PathBuf> {
    let devname = hidraw_path.file_name()?.to_str()?;
    let sysfs_path = PathBuf::from(format!("/sys/class/hidraw/{devname}"));
    sysfs_path.exists().then_some(sysfs_path)
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
    fn uhid_driver_detection_requires_exact_uevent_line() {
        assert!(is_uhid_uevent(
            "DRIVER=uhid\nHID_ID=0003:0000054C:00000DF2\n"
        ));
        assert!(!is_uhid_uevent("PARENT_DRIVER=uhid\n"));
        assert!(!is_uhid_uevent("DRIVER=uhid-extra\n"));
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
