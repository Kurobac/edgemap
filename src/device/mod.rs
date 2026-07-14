mod discovery;
mod hidraw;
mod monitor;
mod permissions;

pub use discovery::{
    find_dualsense, probe_dualsense, DeviceInfo, SonyDeviceKind, SourceTransport, DS4_PID, DS5_PID,
};
#[cfg(test)]
pub use discovery::{DS5_EDGE_PID, SONY_VID};
pub use hidraw::{ioctl_get_feature_report, HidrawDevice};
pub use monitor::{HidrawMonitor, HidrawWait, InputNodesWait};
