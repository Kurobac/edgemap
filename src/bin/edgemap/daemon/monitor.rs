use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dseuhid::{control, shutdown::ShutdownSignal};
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, WatchDescriptor};

const DSEUHID_RUNTIME_DIR: &str = "/run/dseuhid";
const CONTROL_FILE_NAME: &str = "control.sock";
const PROFILE_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Default)]
pub(crate) struct DaemonWake {
    pub(crate) config_changed: bool,
    pub(crate) runtime_changed: bool,
    pub(crate) profile_due: bool,
    pub(crate) shutdown: bool,
}

pub(crate) struct DaemonActivity {
    pub(crate) next_profile_scan: Instant,
    pub(crate) config_changed: bool,
    pub(crate) runtime_changed: bool,
    pub(crate) profile_due: bool,
    pub(crate) shutdown_requested: bool,
}

impl DaemonActivity {
    pub(crate) fn new() -> Self {
        Self {
            next_profile_scan: Instant::now() + PROFILE_INTERVAL,
            config_changed: false,
            runtime_changed: true,
            profile_due: true,
            shutdown_requested: false,
        }
    }
}

pub(crate) struct DaemonMonitor {
    inotify: Inotify,
    pub(crate) config_watch: Option<WatchDescriptor>,
    pub(crate) config_parent_watch: Option<WatchDescriptor>,
    pub(crate) run_watch: Option<WatchDescriptor>,
    pub(crate) runtime_watch: Option<WatchDescriptor>,
    config_dir: PathBuf,
    config_parent_dir: PathBuf,
    config_dir_name: std::ffi::OsString,
    config_name: std::ffi::OsString,
    runtime_dir: PathBuf,
    run_dir: PathBuf,
    runtime_dir_name: std::ffi::OsString,
    runtime_snapshot: RuntimeSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeSnapshot {
    directory_exists: bool,
    socket_exists: bool,
}

impl RuntimeSnapshot {
    fn capture(runtime_dir: &Path) -> Self {
        Self {
            directory_exists: runtime_dir.is_dir(),
            socket_exists: std::fs::symlink_metadata(runtime_dir.join(CONTROL_FILE_NAME)).is_ok(),
        }
    }
}

fn daemon_watch_flags() -> AddWatchFlags {
    AddWatchFlags::IN_CLOSE_WRITE
        | AddWatchFlags::IN_CREATE
        | AddWatchFlags::IN_DELETE
        | AddWatchFlags::IN_MOVED_FROM
        | AddWatchFlags::IN_MOVED_TO
        | AddWatchFlags::IN_DELETE_SELF
        | AddWatchFlags::IN_MOVE_SELF
}

fn run_discovery_flags() -> AddWatchFlags {
    AddWatchFlags::IN_CREATE | AddWatchFlags::IN_MOVED_TO
}

fn config_discovery_flags() -> AddWatchFlags {
    run_discovery_flags() | AddWatchFlags::IN_DELETE_SELF | AddWatchFlags::IN_MOVE_SELF
}

pub(crate) fn watch_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

#[cfg(test)]
pub(crate) fn is_runtime_file(name: &std::ffi::OsStr) -> bool {
    name == CONTROL_FILE_NAME
}

impl DaemonMonitor {
    pub(crate) fn new(config_path: &Path) -> Result<Self, String> {
        Self::new_with_runtime_dir(config_path, Path::new(DSEUHID_RUNTIME_DIR))
    }

    #[cfg(test)]
    fn new_with_runtime_dir(config_path: &Path, runtime_dir: &Path) -> Result<Self, String> {
        Self::new_with_runtime_dir_impl(config_path, runtime_dir, None)
    }

    #[cfg(not(test))]
    fn new_with_runtime_dir(config_path: &Path, runtime_dir: &Path) -> Result<Self, String> {
        Self::new_with_runtime_dir_impl(config_path, runtime_dir)
    }

    #[cfg(test)]
    fn new_with_runtime_dir_after_initial_snapshot(
        config_path: &Path,
        runtime_dir: &Path,
        after_initial_snapshot: &mut dyn FnMut(),
    ) -> Result<Self, String> {
        Self::new_with_runtime_dir_impl(config_path, runtime_dir, Some(after_initial_snapshot))
    }

    fn new_with_runtime_dir_impl(
        config_path: &Path,
        runtime_dir: &Path,
        #[cfg(test)] after_initial_snapshot: Option<&mut dyn FnMut()>,
    ) -> Result<Self, String> {
        let inotify = Inotify::init(InitFlags::IN_CLOEXEC | InitFlags::IN_NONBLOCK)
            .map_err(|e| format!("failed to initialize inotify: {e}"))?;
        let watch_flags = daemon_watch_flags();
        let config_dir = watch_parent(config_path).to_path_buf();
        let config_parent_dir = watch_parent(&config_dir).to_path_buf();
        let config_dir_name = config_dir
            .file_name()
            .ok_or_else(|| {
                format!(
                    "config directory cannot be rediscovered: path={}",
                    config_dir.display()
                )
            })?
            .to_os_string();
        let config_watch = inotify.add_watch(&config_dir, watch_flags).map_err(|e| {
            format!(
                "failed to watch path: path={}, error={e}",
                config_dir.display()
            )
        })?;
        let runtime_dir = runtime_dir.to_path_buf();
        let run_dir = watch_parent(&runtime_dir).to_path_buf();
        let runtime_dir_name = runtime_dir
            .file_name()
            .ok_or_else(|| {
                format!(
                    "runtime directory cannot be discovered: path={}",
                    runtime_dir.display()
                )
            })?
            .to_os_string();
        if !run_dir.is_dir() {
            return Err(format!(
                "runtime parent directory does not exist: path={}",
                run_dir.display()
            ));
        }
        let config_name = config_path
            .file_name()
            .ok_or_else(|| format!("invalid config path: {}", config_path.display()))?
            .to_os_string();
        let runtime_snapshot = RuntimeSnapshot::capture(&runtime_dir);
        #[cfg(test)]
        if let Some(hook) = after_initial_snapshot {
            hook();
        }
        let mut monitor = Self {
            inotify,
            config_watch: Some(config_watch),
            config_parent_watch: None,
            run_watch: None,
            runtime_watch: None,
            config_dir,
            config_parent_dir,
            config_dir_name,
            config_name,
            runtime_dir,
            run_dir,
            runtime_dir_name,
            runtime_snapshot,
        };
        monitor.ensure_runtime_watch()?;
        monitor.ensure_run_watch()?;
        // The directory or socket may appear between the first filesystem
        // check and watch installation. Resynchronization makes that state
        // visible immediately, without waiting for a later inotify event.
        monitor.resync_runtime(false)?;
        Ok(monitor)
    }

    fn ensure_config_watch(&mut self) -> Result<(), String> {
        if self.config_watch.is_some() {
            if self.config_dir.is_dir() {
                return Ok(());
            }
            // The directory can disappear before IN_DELETE_SELF/IN_IGNORED is
            // delivered. Treat the filesystem state as authoritative so the
            // parent discovery watch is installed during this wake cycle.
            self.config_watch = None;
        }

        if self.config_dir.is_dir() {
            self.config_watch = Some(
                self.inotify
                    .add_watch(&self.config_dir, daemon_watch_flags())
                    .map_err(|e| {
                        format!(
                            "failed to watch path: path={}, error={e}",
                            self.config_dir.display()
                        )
                    })?,
            );
            if let Some(parent_watch) = self.config_parent_watch.take() {
                self.inotify.rm_watch(parent_watch).map_err(|e| {
                    format!(
                        "failed to remove path watch: path={}, error={e}",
                        self.config_parent_dir.display()
                    )
                })?;
            }
            return Ok(());
        }

        if self.config_parent_watch.is_none() {
            self.config_parent_watch = Some(
                self.inotify
                    .add_watch(&self.config_parent_dir, config_discovery_flags())
                    .map_err(|e| {
                        format!(
                            "failed to watch config parent: path={}, error={e}",
                            self.config_parent_dir.display()
                        )
                    })?,
            );

            // Close the race between observing the missing directory and
            // installing its temporary parent watch.
            if self.config_dir.is_dir() {
                return self.ensure_config_watch();
            }
        }
        Ok(())
    }

    fn ensure_runtime_watch(&mut self) -> Result<(), String> {
        if self.runtime_watch.is_some() && !self.runtime_dir.is_dir() {
            // A deleted watched directory will deliver IN_IGNORED eventually,
            // but filesystem state is authoritative during resynchronization.
            self.runtime_watch = None;
        }
        if self.runtime_watch.is_none() && self.runtime_dir.is_dir() {
            self.runtime_watch = Some(
                self.inotify
                    .add_watch(&self.runtime_dir, daemon_watch_flags())
                    .map_err(|e| {
                        format!(
                            "failed to watch path: path={}, error={e}",
                            self.runtime_dir.display()
                        )
                    })?,
            );
            if let Some(run_watch) = self.run_watch.take() {
                self.inotify.rm_watch(run_watch).map_err(|e| {
                    format!(
                        "failed to remove path watch: path={}, error={e}",
                        self.run_dir.display()
                    )
                })?;
            }
        }
        Ok(())
    }

    fn ensure_run_watch(&mut self) -> Result<(), String> {
        if self.runtime_watch.is_none() && self.run_watch.is_none() {
            self.run_watch = Some(
                self.inotify
                    .add_watch(&self.run_dir, run_discovery_flags())
                    .map_err(|e| {
                        format!(
                            "failed to watch path: path={}, error={e}",
                            self.run_dir.display()
                        )
                    })?,
            );
            // Close the check-before-watch window just as the config watcher
            // does: a present runtime directory must win over the parent watch.
            if self.runtime_dir.is_dir() {
                self.ensure_runtime_watch()?;
            }
        }
        Ok(())
    }

    fn resync_runtime(&mut self, control_connected: bool) -> Result<bool, String> {
        self.ensure_runtime_watch()?;
        self.ensure_run_watch()?;
        self.ensure_runtime_watch()?;

        let current = RuntimeSnapshot::capture(&self.runtime_dir);
        let changed = current != self.runtime_snapshot;
        self.runtime_snapshot = current;
        Ok(changed || (!control_connected && current.socket_exists))
    }

    pub(crate) fn wait(
        &mut self,
        deadline: Instant,
        shutdown: &ShutdownSignal,
        control_client: Option<&control::ControlClient>,
    ) -> Result<DaemonWake, String> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as u32;
        let mut fds = vec![
            PollFd::new(self.inotify.as_fd(), PollFlags::POLLIN),
            PollFd::new(shutdown.as_fd(), PollFlags::POLLIN),
        ];
        if let Some(client) = control_client {
            fds.push(PollFd::new(
                client.as_fd(),
                PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP,
            ));
        }
        match poll(
            &mut fds,
            PollTimeout::try_from(timeout_ms).unwrap_or(PollTimeout::MAX),
        ) {
            Ok(0) => {
                drop(fds);
                return Ok(DaemonWake {
                    runtime_changed: self.resync_runtime(control_client.is_some())?,
                    profile_due: true,
                    ..Default::default()
                });
            }
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => {
                drop(fds);
                return Ok(DaemonWake {
                    runtime_changed: self.resync_runtime(control_client.is_some())?,
                    ..Default::default()
                });
            }
            Err(e) => return Err(format!("inotify poll failed: {e}")),
        }

        let mut wake = DaemonWake::default();
        let inotify_events = fds[0].revents().unwrap_or(PollFlags::empty());
        let shutdown_events = fds[1].revents().unwrap_or(PollFlags::empty());
        let control_events = fds
            .get(2)
            .and_then(|fd| fd.revents())
            .unwrap_or(PollFlags::empty());
        drop(fds);
        let failure = PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL;
        if inotify_events.intersects(failure) {
            return Err("inotify poll reported a failure".to_string());
        }
        if shutdown_events.intersects(failure) {
            return Err("shutdown signal fd poll reported a failure".to_string());
        }
        if control_events.intersects(failure) || control_events.contains(PollFlags::POLLIN) {
            wake.runtime_changed = true;
        }
        if shutdown_events.contains(PollFlags::POLLIN) {
            shutdown
                .consume()
                .map_err(|e| format!("failed to read shutdown signal: {e}"))?;
            wake.shutdown = true;
            return Ok(wake);
        }
        if !inotify_events.contains(PollFlags::POLLIN) {
            wake.runtime_changed |= self.resync_runtime(control_client.is_some())?;
            wake.profile_due = Instant::now() >= deadline;
            return Ok(wake);
        }
        let events = self
            .inotify
            .read_events()
            .map_err(|e| format!("failed to read inotify events: {e}"))?;
        for event in events {
            if event.mask.contains(AddWatchFlags::IN_Q_OVERFLOW) {
                log::warn!(
                    "inotify event queue overflowed; daemon state resynchronization requested"
                );
                wake.config_changed = true;
                wake.runtime_changed = true;
                continue;
            }
            if self.config_watch == Some(event.wd)
                && event.name.as_deref() == Some(self.config_name.as_os_str())
            {
                wake.config_changed = true;
            }
            if self.config_watch == Some(event.wd)
                && event.mask.intersects(
                    AddWatchFlags::IN_DELETE_SELF
                        | AddWatchFlags::IN_MOVE_SELF
                        | AddWatchFlags::IN_IGNORED,
                )
            {
                wake.config_changed = true;
                self.config_watch = None;
            }
            if self.config_parent_watch == Some(event.wd)
                && event.name.as_deref() == Some(self.config_dir_name.as_os_str())
            {
                wake.config_changed = true;
                self.ensure_config_watch()?;
            }
            if self.config_parent_watch == Some(event.wd)
                && event.mask.intersects(
                    AddWatchFlags::IN_DELETE_SELF
                        | AddWatchFlags::IN_MOVE_SELF
                        | AddWatchFlags::IN_IGNORED,
                )
            {
                return Err(format!(
                    "config parent directory watch lost: {}",
                    self.config_parent_dir.display()
                ));
            }
            if self.run_watch == Some(event.wd)
                && event.name.as_deref() == Some(self.runtime_dir_name.as_os_str())
            {
                self.ensure_runtime_watch()?;
            }
            if self.runtime_watch == Some(event.wd)
                && event.mask.intersects(
                    AddWatchFlags::IN_DELETE_SELF
                        | AddWatchFlags::IN_MOVE_SELF
                        | AddWatchFlags::IN_IGNORED,
                )
            {
                self.runtime_watch = None;
            }
        }
        self.ensure_config_watch()?;
        wake.runtime_changed |= self.resync_runtime(control_client.is_some())?;
        wake.profile_due = Instant::now() >= deadline;
        Ok(wake)
    }
}

pub(crate) fn wait_for_daemon_activity(
    monitor: &mut DaemonMonitor,
    shutdown: &ShutdownSignal,
    control_client: Option<&control::ControlClient>,
    activity: &mut DaemonActivity,
) -> Result<(), String> {
    let wake = monitor.wait(activity.next_profile_scan, shutdown, control_client)?;
    activity.config_changed |= wake.config_changed;
    activity.runtime_changed |= wake.runtime_changed;
    activity.shutdown_requested |= wake.shutdown;
    if wake.profile_due {
        activity.profile_due = true;
        activity.next_profile_scan = Instant::now() + PROFILE_INTERVAL;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            Path::new("/tmp").join(format!("edm-{name}-{:x}-{unique:x}", std::process::id()));
        let config_dir = root.join("config");
        let runtime_dir = root.join("run").join("dseuhid");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(runtime_dir.parent().unwrap()).unwrap();
        let config_path = config_dir.join("edgemap.toml");
        std::fs::write(&config_path, "config = \"default.toml\"\n").unwrap();
        (root, config_path, runtime_dir)
    }

    #[test]
    fn runtime_resync_closes_the_missing_directory_watch_race() {
        let (root, config_path, runtime_dir) = test_paths("race");
        let mut server = None;
        let mut monitor = {
            let mut create_runtime = || {
                server = Some(
                    control::ControlServer::bind(
                        &runtime_dir,
                        control::ControlState {
                            uhid_ready: true,
                            needs_config: false,
                            bt_haptics: None,
                        },
                    )
                    .unwrap(),
                );
            };
            DaemonMonitor::new_with_runtime_dir_after_initial_snapshot(
                &config_path,
                &runtime_dir,
                &mut create_runtime,
            )
            .unwrap()
        };

        assert!(server.is_some());
        assert!(monitor.runtime_watch.is_some());
        assert!(monitor.run_watch.is_none());
        assert_eq!(
            monitor.runtime_snapshot,
            RuntimeSnapshot {
                directory_exists: true,
                socket_exists: true,
            }
        );
        assert!(!monitor.resync_runtime(true).unwrap());

        drop(server);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn wait_timeout_does_not_report_runtime_change_for_a_connected_client() {
        let (root, config_path, runtime_dir) = test_paths("stable");
        let mut server = control::ControlServer::bind(
            &runtime_dir,
            control::ControlState {
                uhid_ready: true,
                needs_config: false,
                bt_haptics: None,
            },
        )
        .unwrap();
        let mut monitor = DaemonMonitor::new_with_runtime_dir(&config_path, &runtime_dir).unwrap();
        let client = control::ControlClient::connect(&runtime_dir.join(CONTROL_FILE_NAME)).unwrap();
        assert!(server.drain_requests().unwrap().is_empty());
        assert!(matches!(
            client.receive().unwrap(),
            Some(control::ServerPacket::Hello(_))
        ));

        let shutdown = ShutdownSignal::new().unwrap();
        let wake = monitor
            .wait(
                Instant::now() + Duration::from_millis(20),
                &shutdown,
                Some(&client),
            )
            .unwrap();

        assert!(wake.profile_due);
        assert!(!wake.runtime_changed);

        drop(client);
        drop(server);
        std::fs::remove_dir_all(root).unwrap();
    }
}
