use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::path::{Path, PathBuf};

use log::debug;
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use udev::{Device as UdevDevice, Enumerator, EventType, MonitorBuilder, MonitorSocket};

use crate::shutdown::ShutdownSignal;

use super::discovery::{find_sysfs_hidraw, is_hidraw_path};

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

pub(super) struct AssociatedInputNode {
    pub(super) name: String,
    pub(super) dev_path: Option<PathBuf>,
    pub(super) initialized: bool,
}

impl AssociatedInputNode {
    pub(super) fn is_ready(&self) -> bool {
        self.initialized && self.dev_path.as_ref().is_some_and(|path| path.exists())
    }
}

pub(super) fn associated_input_nodes(hidraw_sysfs: &Path) -> io::Result<Vec<AssociatedInputNode>> {
    let hid_parent = std::fs::canonicalize(hidraw_sysfs.join("device"))?;
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
            "waiting for associated input nodes: hidraw={}",
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
        debug!(
            "waiting for udev to initialize input nodes: nodes={}",
            pending.join(", ")
        );
        return Ok(InputNodesState::Pending);
    }
    Ok(InputNodesState::Ready)
}
