use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dseuhid::{control, shutdown::ShutdownSignal};
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, WatchDescriptor};

const DSEUHID_RUNTIME_DIR: &str = "/run/dseuhid";
const RUN_DIR: &str = "/run";
const CONTROL_FILE_NAME: &str = "control.sock";
const PROFILE_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Default)]
pub(crate) struct DaemonWake {
    pub(crate) config_changed: bool,
    pub(crate) runtime_changed: bool,
    pub(crate) profile_due: bool,
    pub(crate) shutdown: bool,
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

pub(crate) fn is_runtime_file(name: &std::ffi::OsStr) -> bool {
    name == CONTROL_FILE_NAME
}

impl DaemonMonitor {
    pub(crate) fn new(config_path: &Path) -> Result<Self, String> {
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
        let runtime_exists = Path::new(DSEUHID_RUNTIME_DIR).is_dir();
        let runtime_watch = if runtime_exists {
            Some(
                inotify
                    .add_watch(DSEUHID_RUNTIME_DIR, watch_flags)
                    .map_err(|e| {
                        format!("failed to watch path: path={DSEUHID_RUNTIME_DIR}, error={e}")
                    })?,
            )
        } else {
            None
        };
        let run_watch = if runtime_exists {
            None
        } else {
            Some(
                inotify
                    .add_watch(RUN_DIR, run_discovery_flags())
                    .map_err(|e| format!("failed to watch path: path={RUN_DIR}, error={e}"))?,
            )
        };
        let config_name = config_path
            .file_name()
            .ok_or_else(|| format!("invalid config path: {}", config_path.display()))?
            .to_os_string();
        Ok(Self {
            inotify,
            config_watch: Some(config_watch),
            config_parent_watch: None,
            run_watch,
            runtime_watch,
            config_dir,
            config_parent_dir,
            config_dir_name,
            config_name,
        })
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
        if self.runtime_watch.is_none() && Path::new(DSEUHID_RUNTIME_DIR).is_dir() {
            self.runtime_watch = Some(
                self.inotify
                    .add_watch(DSEUHID_RUNTIME_DIR, daemon_watch_flags())
                    .map_err(|e| {
                        format!("failed to watch path: path={DSEUHID_RUNTIME_DIR}, error={e}")
                    })?,
            );
            if let Some(run_watch) = self.run_watch.take() {
                self.inotify.rm_watch(run_watch).map_err(|e| {
                    format!("failed to remove path watch: path={RUN_DIR}, error={e}")
                })?;
            }
        }
        Ok(())
    }

    fn ensure_run_watch(&mut self) -> Result<(), String> {
        if self.runtime_watch.is_none() && self.run_watch.is_none() {
            self.run_watch = Some(
                self.inotify
                    .add_watch(RUN_DIR, run_discovery_flags())
                    .map_err(|e| format!("failed to watch path: path={RUN_DIR}, error={e}"))?,
            );
        }
        Ok(())
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
                return Ok(DaemonWake {
                    profile_due: true,
                    ..Default::default()
                })
            }
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => return Ok(DaemonWake::default()),
            Err(e) => return Err(format!("inotify poll failed: {e}")),
        }

        let mut wake = DaemonWake::default();
        let inotify_events = fds[0].revents().unwrap_or(PollFlags::empty());
        let shutdown_events = fds[1].revents().unwrap_or(PollFlags::empty());
        let control_events = fds
            .get(2)
            .and_then(|fd| fd.revents())
            .unwrap_or(PollFlags::empty());
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
                && event.name.as_deref() == Some(std::ffi::OsStr::new("dseuhid"))
            {
                wake.runtime_changed = true;
                self.ensure_runtime_watch()?;
            }
            if self.runtime_watch == Some(event.wd)
                && event.name.as_deref().is_some_and(is_runtime_file)
            {
                wake.runtime_changed = true;
            }
            if self.runtime_watch == Some(event.wd)
                && event.mask.intersects(
                    AddWatchFlags::IN_DELETE_SELF
                        | AddWatchFlags::IN_MOVE_SELF
                        | AddWatchFlags::IN_IGNORED,
                )
            {
                wake.runtime_changed = true;
                self.runtime_watch = None;
            }
        }
        self.ensure_config_watch()?;
        self.ensure_runtime_watch()?;
        self.ensure_run_watch()?;
        wake.profile_due = Instant::now() >= deadline;
        Ok(wake)
    }
}

pub(crate) fn wait_for_daemon_activity(
    monitor: &mut DaemonMonitor,
    shutdown: &ShutdownSignal,
    control_client: Option<&control::ControlClient>,
    next_profile_scan: &mut Instant,
    config_changed: &mut bool,
    runtime_changed: &mut bool,
    profile_due: &mut bool,
    shutdown_requested: &mut bool,
) -> Result<(), String> {
    let wake = monitor.wait(*next_profile_scan, shutdown, control_client)?;
    *config_changed |= wake.config_changed;
    *runtime_changed |= wake.runtime_changed;
    *shutdown_requested |= wake.shutdown;
    if wake.profile_due {
        *profile_due = true;
        *next_profile_scan = Instant::now() + PROFILE_INTERVAL;
    }
    Ok(())
}
