use std::env;
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, WatchDescriptor};
mod control_session;
mod paths;
mod profile;

use control_session::*;
use paths::*;
use profile::{find_matching_profile, ProfileConfig};
#[cfg(test)]
use profile::{profile_matches, ProcessSnapshot};

use dseuhid::{config, control, shutdown};
use shutdown::{unblock_shutdown_signals_in_child, ShutdownSignal};

const DSEUHID_RUNTIME_DIR: &str = "/run/dseuhid";
const RUN_DIR: &str = "/run";
const CONTROL_FILE_NAME: &str = "control.sock";
const EDGEMAP_CONFIG_FILE: &str = "edgemap.toml";
const DEFAULT_CONFIG_FILE: &str = "default.toml";
const PROFILE_INTERVAL: Duration = Duration::from_secs(3);

const USAGE: &str = concat!(
    "edgemap — configuration CLI for dseuhid\n",
    "\n",
    "Usage: edgemap <COMMAND> [ARGS]\n",
    "\n",
    "Commands:\n",
    "  v, validate [PATH]           Validate one config or all configs\n",
    "  cc, create-config [PATH]     Create the default config; print it if PATH is omitted\n",
    "  sc, switch-config <PATH>     Switch to another config\n",
    "  d, daemon [--config <PATH>]  Watch dseuhid and manage config selection\n",
    "  help                         Print help\n",
);

fn print_usage(to_stdout: bool) {
    if to_stdout {
        print!("{USAGE}");
    } else {
        eprint!("{USAGE}");
    }
}

#[derive(Default)]
struct DaemonWake {
    config_changed: bool,
    runtime_changed: bool,
    profile_due: bool,
    shutdown: bool,
}

struct DaemonMonitor {
    inotify: Inotify,
    config_watch: Option<WatchDescriptor>,
    config_parent_watch: Option<WatchDescriptor>,
    run_watch: Option<WatchDescriptor>,
    runtime_watch: Option<WatchDescriptor>,
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

fn watch_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn is_runtime_file(name: &std::ffi::OsStr) -> bool {
    name == CONTROL_FILE_NAME
}

fn needs_config_became_true(previous: Option<bool>, current: bool) -> bool {
    current && previous != Some(true)
}

impl DaemonMonitor {
    fn new(config_path: &Path) -> Result<Self, String> {
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
            return Ok(());
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

    fn wait(
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

fn wait_for_daemon_activity(
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

fn send_notification(summary: &str, body: &str) {
    let mut command = std::process::Command::new("notify-send");
    command
        .args(["-a", "edgemap", summary, body])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    unblock_shutdown_signals_in_child(&mut command);
    match command.spawn() {
        Ok(child) => {
            if let Err(error) = reap_child(child) {
                log::warn!("failed to start notify-send child reaper: {error}");
            }
        }
        Err(error) => log::debug!("failed to start notify-send: {error}"),
    }
}

fn reap_child(mut child: std::process::Child) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("edgemap-child-reaper".to_string())
        .spawn(move || {
            if let Err(error) = child.wait() {
                log::debug!("failed to reap notify-send child: {error}");
            }
        })
}

fn cmd_validate(args: &[String]) -> ! {
    let load = |path: &str| config::ActiveConfig::read(path).and_then(|config| config.parse());

    if args.len() > 3 {
        eprintln!("error: too many arguments");
        eprintln!("Usage: edgemap validate [PATH]");
        std::process::exit(1);
    }

    if args.len() == 3 {
        let path = &args[2];
        let cfg = match load(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        };
        match config::validate(&cfg) {
            Ok(()) => {
                if cfg.buttons.is_empty() {
                    println!("Valid: {path} (passthrough only)");
                } else {
                    println!("Valid: {path}");
                }
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
    }

    // No path given — validate all configs in ~/.config/edgemap/
    let dir = edgemap_config_dir().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    if !dir.exists() {
        println!("No config directory: {}", dir.display());
        std::process::exit(0);
    }
    let mut ok = 0;
    let mut fail = 0;
    let mut entries: Vec<_> = match std::fs::read_dir(&dir) {
        Ok(d) => d
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.ends_with(".toml") && n != EDGEMAP_CONFIG_FILE)
            })
            .collect(),
        Err(e) => {
            eprintln!(
                "error: failed to read config directory '{}': {e}",
                dir.display()
            );
            std::process::exit(1);
        }
    };
    entries.sort_by_key(|e| e.file_name());

    println!("Checking configs in {}", dir.display());
    for entry in entries {
        let path = entry.path();
        let display = entry.file_name().to_string_lossy().into_owned();
        match load(path.to_str().unwrap()) {
            Ok(cfg) => match config::validate(&cfg) {
                Ok(()) => {
                    let note = if cfg.buttons.is_empty() {
                        " (passthrough only)"
                    } else {
                        ""
                    };
                    println!("  OK    {display}{note}");
                    ok += 1;
                }
                Err(e) => {
                    println!("  FAIL  {display}: {e}");
                    fail += 1;
                }
            },
            Err(e) => {
                println!("  FAIL  {display}: {e}");
                fail += 1;
            }
        }
    }
    let total = ok + fail;
    println!("Summary: {ok}/{total} valid");
    std::process::exit(if fail > 0 { 1 } else { 0 });
}

fn cmd_create_config(args: &[String]) -> ! {
    if args.len() > 3 {
        eprintln!("error: too many arguments");
        eprintln!("Usage: edgemap create-config [PATH]");
        std::process::exit(1);
    }
    let content = config::default_content();
    if args.len() >= 3 {
        let path = &args[2];
        if Path::new(path).exists() {
            eprintln!("error: config already exists: {path}");
            std::process::exit(1);
        }
        if let Some(parent) = Path::new(path).parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("error: failed to create parent directory for '{path}': {e}");
                std::process::exit(1);
            }
        }
        if let Err(e) = std::fs::write(path, content) {
            eprintln!("error: failed to write config '{path}': {e}");
            std::process::exit(1);
        }
        println!("Created: {path}");
    } else {
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        if let Err(e) = handle.write_all(content.as_bytes()) {
            eprintln!("error: failed to write config to stdout: {e}");
            std::process::exit(1);
        }
    }
    std::process::exit(0);
}

fn send_control_command(request: control::ControlRequest) -> ! {
    let success = match &request {
        control::ControlRequest::SwitchConfig(active_config) => {
            format!("Config switched: {}", active_config.source())
        }
    };
    match send_control_request(&request) {
        Ok(_) => {
            println!("{success}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_switch_config(args: &[String]) -> ! {
    if args.len() < 3 {
        eprintln!("error: command 'switch-config' requires a path");
        eprintln!("Usage: edgemap switch-config <PATH>");
        std::process::exit(1);
    }
    if args.len() > 3 {
        eprintln!("error: too many arguments");
        eprintln!("Usage: edgemap switch-config <PATH>");
        std::process::exit(1);
    }
    let path = &args[2];
    let path_str = if Path::new(path).is_absolute() {
        path.clone()
    } else if path.starts_with('.') {
        std::fs::canonicalize(path)
            .unwrap_or_else(|e| {
                eprintln!("error: failed to resolve config path '{path}': {e}");
                std::process::exit(1);
            })
            .to_string_lossy()
            .to_string()
    } else if path.starts_with('~') {
        resolve_config_path(path, Path::new("")).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        })
    } else {
        edgemap_config_dir()
            .and_then(|dir| resolve_config_path(path, &dir))
            .unwrap_or_else(|e| {
                eprintln!("error: {e}");
                std::process::exit(1);
            })
    };
    let active_config = match config::ActiveConfig::read(&path_str) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let cfg = match active_config.parse() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = config::validate(&cfg) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
    send_control_command(control::ControlRequest::SwitchConfig(active_config))
}

struct DaemonState {
    base_config: String,
    base_config_raw: String,
    profiles: Vec<(String, ProfileConfig)>,
    valid_profiles: Vec<(String, String)>,
    dir: PathBuf,
}

fn extract_profile_order(raw: &str) -> Vec<String> {
    raw.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("[profiles.")
                .and_then(|rest| rest.strip_suffix(']'))
                .map(|s| s.to_string())
        })
        .collect()
}

fn load_edgemap_config(path: &Path) -> Result<DaemonState, String> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "failed to read edgemap config: path={}, error={e}",
            path.display()
        )
    })?;
    let root: toml::Value = toml::from_str(&content).map_err(|e| {
        format!(
            "failed to parse edgemap config: path={}, error={e}",
            path.display()
        )
    })?;
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let base_config_raw = root
        .get("config")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_CONFIG_FILE)
        .to_string();
    let base_config = resolve_config_path(&base_config_raw, &dir)?;

    // defer validation of base/default config to daemon loop (pre-injection)

    let mut profiles: Vec<(String, ProfileConfig)> = Vec::new();
    if let Some(t) = root.get("profiles").and_then(|v| v.as_table()) {
        for (name, val) in t.iter() {
            match val.clone().try_into::<ProfileConfig>() {
                Ok(mut cfg) => {
                    cfg.match_process = cfg.match_process.to_lowercase();
                    cfg.match_cmdline = cfg.match_cmdline.to_lowercase();
                    profiles.push((name.clone(), cfg));
                }
                Err(e) => log::warn!("profile skipped: name={name}, error={e}"),
            }
        }
    }

    // sort by declaration order in the TOML file
    let decl_order = extract_profile_order(&content);
    profiles.sort_by_key(|(name, _)| {
        decl_order
            .iter()
            .position(|n| n == name)
            .unwrap_or(usize::MAX)
    });

    let mut valid_profiles: Vec<(String, String)> = Vec::new();
    for (name, pcfg) in &profiles {
        let p_path = resolve_config_path(&pcfg.config, &dir)?;
        if pcfg.match_process.is_empty() && pcfg.match_cmdline.is_empty() {
            log::warn!("profile skipped: name={name}, reason=no match criteria");
            continue;
        }
        // defer config existence/validation to daemon loop (pre-injection)
        valid_profiles.push((name.clone(), p_path));
    }

    Ok(DaemonState {
        base_config,
        base_config_raw,
        profiles,
        valid_profiles,
        dir,
    })
}

fn cmd_daemon(args: &[String]) -> ! {
    let mut config_arg: Option<&str> = None;

    // parse optional --config <path> from args
    let mut i = 2;
    while i < args.len() {
        if args[i] == "--config" && i + 1 < args.len() {
            config_arg = Some(&args[i + 1]);
            i += 1;
        } else {
            eprintln!("error: unknown argument '{}'", args[i]);
            eprintln!("Usage: edgemap daemon [--config <PATH>]");
            std::process::exit(1);
        }
        i += 1;
    }

    let edgemap_config_path = match config_arg {
        Some(path) if Path::new(path).is_absolute() => Ok(PathBuf::from(path)),
        Some(path) if path.starts_with('~') => {
            resolve_config_path(path, Path::new("")).map(PathBuf::from)
        }
        Some(path) => edgemap_config_dir()
            .and_then(|dir| resolve_config_path(path, &dir))
            .map(PathBuf::from),
        None => edgemap_config_dir().map(|dir| dir.join(EDGEMAP_CONFIG_FILE)),
    }
    .unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let state_dir = edgemap_state_dir().unwrap_or_else(|e| {
        log::error!("failed to resolve state directory: {e}");
        std::process::exit(1);
    });
    let _daemon_lock =
        control::DaemonLock::acquire_named(&state_dir, "edgemap.lock", "edgemap daemon")
            .unwrap_or_else(|e| {
                log::error!("failed to acquire edgemap daemon lock: {e}");
                std::process::exit(1);
            });

    let dir = edgemap_config_path.parent().unwrap_or(Path::new("."));

    if !edgemap_config_path.exists() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::error!(
                "failed to create config directory: path={}, error={e}",
                dir.display()
            );
            std::process::exit(1);
        }
        let default_toml_content = config::default_content();
        let default_remap_path = dir.join(DEFAULT_CONFIG_FILE);
        if !default_remap_path.exists() {
            if let Err(e) = std::fs::write(&default_remap_path, default_toml_content) {
                log::error!(
                    "failed to write config: path={}, error={e}",
                    default_remap_path.display()
                );
                std::process::exit(1);
            }
            log::info!("config created: path={}", default_remap_path.display());
        }
        let edgemap_toml = format!("config = \"{DEFAULT_CONFIG_FILE}\"\n");
        if let Err(e) = std::fs::write(&edgemap_config_path, edgemap_toml) {
            log::error!(
                "failed to write config: path={}, error={e}",
                edgemap_config_path.display()
            );
            std::process::exit(1);
        }
        log::info!("config created: path={}", edgemap_config_path.display());
    }

    let config_path = edgemap_config_path.clone();
    let mut state = match load_edgemap_config(&config_path) {
        Ok(s) => s,
        Err(e) => {
            log::error!("failed to load edgemap config: {e}");
            std::process::exit(1);
        }
    };

    let shutdown = ShutdownSignal::new().unwrap_or_else(|e| {
        log::error!("failed to initialize signal handling: {e}");
        std::process::exit(1);
    });

    // SIGPIPE keeps its existing default behavior; SIGINT/SIGTERM use signalfd.
    unsafe {
        let handler = libc::SIG_DFL;
        let _ = libc::signal(libc::SIGPIPE, handler);
    }

    log::info!("edgemap daemon started");
    log::info!("edgemap config: path={}", config_path.display());

    let mut monitor = DaemonMonitor::new(&config_path).unwrap_or_else(|e| {
        log::error!("failed to initialize daemon monitor: {e}");
        std::process::exit(1);
    });

    let mut current_config = String::new();
    let mut control_client: Option<control::ControlClient> = None;
    let mut control_state: Option<control::ControlState> = None;
    let mut warned_not_running = false;
    let mut config_changed = false;
    let mut runtime_changed = true;
    let mut profile_due = true;
    let mut shutdown_requested = false;
    let mut next_profile_scan = Instant::now() + PROFILE_INTERVAL;

    while !shutdown_requested {
        if config_changed {
            config_changed = false;
            match load_edgemap_config(&config_path) {
                Ok(s) => {
                    state = s;
                    current_config.clear();
                    profile_due = true;
                    log::info!("edgemap config reloaded: path={}", config_path.display());
                }
                Err(e) => {
                    log::error!("failed to reload edgemap config; previous config retained: {e}")
                }
            }
        }

        if runtime_changed {
            runtime_changed = false;
            let previous_state = control_state;
            let was_alive = control_client.is_some();
            let mut disconnect_reason = None;

            if let Some(client) = control_client.as_ref() {
                match drain_control_state(client) {
                    Ok(Some(state)) => control_state = Some(state),
                    Ok(None) => {}
                    Err(e) => {
                        disconnect_reason = Some(e);
                        control_client = None;
                        control_state = None;
                    }
                }
            } else {
                match connect_control() {
                    Ok((client, state)) => {
                        control_client = Some(client);
                        control_state = Some(state);
                        warned_not_running = false;
                    }
                    Err(e) => {
                        if !warned_not_running {
                            log::info!("waiting for dseuhid: {e}");
                            warned_not_running = true;
                        }
                    }
                }
            }

            if control_client.is_none() {
                if previous_state.is_some_and(|state| state.uhid_ready) {
                    log::info!("virtual HID device unavailable");
                }
                if was_alive {
                    log::warn!(
                        "dseuhid control connection lost: reason={}",
                        disconnect_reason
                            .as_deref()
                            .unwrap_or("control socket closed")
                    );
                }
            } else if let Some(state) = control_state {
                if !was_alive {
                    log::info!("dseuhid control connection established");
                }
                let previous_ready = previous_state.is_some_and(|old| old.uhid_ready);
                if state.uhid_ready && !previous_ready {
                    log::info!("virtual HID device ready");
                } else if !state.uhid_ready && previous_ready {
                    log::info!("virtual HID device unavailable");
                }
                let previous_needs = previous_state.map(|old| old.needs_config);
                if needs_config_became_true(previous_needs, state.needs_config) {
                    current_config.clear();
                    profile_due = true;
                }
            }
        }

        if !control_state.is_some_and(|state| state.uhid_ready) {
            if let Err(e) = wait_for_daemon_activity(
                &mut monitor,
                &shutdown,
                control_client.as_ref(),
                &mut next_profile_scan,
                &mut config_changed,
                &mut runtime_changed,
                &mut profile_due,
                &mut shutdown_requested,
            ) {
                log::error!("daemon wait failed: {e}");
                break;
            }
            continue;
        }

        if !profile_due {
            if let Err(e) = wait_for_daemon_activity(
                &mut monitor,
                &shutdown,
                control_client.as_ref(),
                &mut next_profile_scan,
                &mut config_changed,
                &mut runtime_changed,
                &mut profile_due,
                &mut shutdown_requested,
            ) {
                log::error!("daemon wait failed: {e}");
                break;
            }
            continue;
        }
        profile_due = false;

        let wanted = if state.valid_profiles.is_empty() {
            state.base_config.clone()
        } else {
            let valid: Vec<_> = state
                .profiles
                .iter()
                .filter(|(name, _)| state.valid_profiles.iter().any(|(vn, _)| vn == name))
                .cloned()
                .collect();
            match find_matching_profile(&valid, &state.dir, &state.base_config_raw) {
                Ok(Some(path)) => path,
                Ok(None) => state.base_config.clone(),
                Err(e) => {
                    log::error!("failed to resolve profile config: {e}");
                    if let Err(wait_error) = wait_for_daemon_activity(
                        &mut monitor,
                        &shutdown,
                        control_client.as_ref(),
                        &mut next_profile_scan,
                        &mut config_changed,
                        &mut runtime_changed,
                        &mut profile_due,
                        &mut shutdown_requested,
                    ) {
                        log::error!("daemon wait failed: {wait_error}");
                        break;
                    }
                    continue;
                }
            }
        };

        if wanted != current_config {
            // validate before injecting — catches profiles configured before
            // their config files are created, or invalid save states
            let load_valid = |p: &str| -> Option<config::ActiveConfig> {
                if !Path::new(p).exists() {
                    log::warn!("config not found: path={p}");
                    return None;
                }
                match config::ActiveConfig::read(p) {
                    Ok(active_config) => match active_config.parse() {
                        Ok(cfg) => {
                            if let Err(e) = config::validate(&cfg) {
                                log::warn!("config validation failed: path={p}, error={e}");
                                None
                            } else {
                                Some(active_config)
                            }
                        }
                        Err(e) => {
                            log::warn!("failed to parse config: path={p}, error={e}");
                            None
                        }
                    },
                    Err(e) => {
                        log::warn!("failed to load config: path={p}, error={e}");
                        None
                    }
                }
            };

            let mut target = wanted.clone();
            let active_config = if let Some(active_config) = load_valid(&target) {
                active_config
            } else {
                // profile config failed — try base_config as fallback
                if target != state.base_config {
                    log::warn!("profile config invalid; using default config");
                    target = state.base_config.clone();
                    let Some(active_config) = load_valid(&target) else {
                        log::warn!("default config also invalid; previous config retained");
                        if let Err(wait_error) = wait_for_daemon_activity(
                            &mut monitor,
                            &shutdown,
                            control_client.as_ref(),
                            &mut next_profile_scan,
                            &mut config_changed,
                            &mut runtime_changed,
                            &mut profile_due,
                            &mut shutdown_requested,
                        ) {
                            log::error!("daemon wait failed: {wait_error}");
                            break;
                        }
                        continue;
                    };
                    active_config
                } else {
                    // base_config itself is invalid — just warn, don't spam
                    log::warn!("default config invalid; previous config retained");
                    if let Err(wait_error) = wait_for_daemon_activity(
                        &mut monitor,
                        &shutdown,
                        control_client.as_ref(),
                        &mut next_profile_scan,
                        &mut config_changed,
                        &mut runtime_changed,
                        &mut profile_due,
                        &mut shutdown_requested,
                    ) {
                        log::error!("daemon wait failed: {wait_error}");
                        break;
                    }
                    continue;
                }
            };

            let request = control::ControlRequest::SwitchConfig(active_config);
            let result = match (control_client.as_ref(), control_state.as_mut()) {
                (Some(client), Some(control_state)) => {
                    send_daemon_control_request(client, &request, &shutdown, control_state)
                }
                _ => Err(DaemonRequestError::Failed(
                    "dseuhid control connection is unavailable".to_string(),
                )),
            };
            match result {
                Ok(()) => {
                    let label = state
                        .profiles
                        .iter()
                        .find(|(_, pc)| {
                            resolve_config_path(&pc.config, &state.dir).as_deref()
                                == Ok(target.as_str())
                        })
                        .map(|(name, _)| format!("profile '{name}'"))
                        .unwrap_or_else(|| "default config".to_string());
                    log::info!("config applied: source={label}");
                    log::info!("config path: path={target}");
                    send_notification("edgemap", &format!("Switched to {label}"));
                    current_config = target;
                }
                Err(DaemonRequestError::Shutdown) => {
                    break;
                }
                Err(DaemonRequestError::Failed(e)) => {
                    log::warn!("dseuhid control request failed: {e}");
                    runtime_changed = true;
                }
            }
        }
        if let Err(e) = wait_for_daemon_activity(
            &mut monitor,
            &shutdown,
            control_client.as_ref(),
            &mut next_profile_scan,
            &mut config_changed,
            &mut runtime_changed,
            &mut profile_due,
            &mut shutdown_requested,
        ) {
            log::error!("daemon wait failed: {e}");
            break;
        }
    }

    log::info!("edgemap daemon stopped");
    std::process::exit(0);
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage(false);
        std::process::exit(1);
    }
    match args[1].as_str() {
        "v" | "validate" => cmd_validate(&args),
        "cc" | "create-config" => cmd_create_config(&args),
        "sc" | "switch-config" => cmd_switch_config(&args),
        "d" | "daemon" => cmd_daemon(&args),
        "help" | "--help" | "-h" => {
            print_usage(true);
            std::process::exit(0);
        }
        _ => {
            eprintln!("error: unknown command '{}'", args[1]);
            eprintln!("hint: run 'edgemap help' for usage");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn usage_uses_conventional_placeholders() {
        assert!(USAGE.contains("Usage: edgemap <COMMAND> [ARGS]"));
        assert!(USAGE.contains("switch-config <PATH>"));
        assert!(!USAGE.contains("  r, reload"));
        assert!(!USAGE.contains("<path>"));
    }

    #[test]
    fn absolute_xdg_path_is_used_without_home() {
        assert_eq!(
            resolve_xdg_dir(Some(Path::new("/tmp/xdg")), None, Path::new(".config")),
            Ok(PathBuf::from("/tmp/xdg/edgemap"))
        );
    }

    #[test]
    fn invalid_xdg_paths_fall_back_to_home() {
        for xdg in [Path::new(""), Path::new("relative/path")] {
            assert_eq!(
                resolve_xdg_dir(Some(xdg), Some("/home/test"), Path::new(".config")),
                Ok(PathBuf::from("/home/test/.config/edgemap"))
            );
        }
    }

    #[test]
    fn missing_home_rejects_xdg_fallback() {
        assert!(resolve_xdg_dir(None, None, Path::new(".local/state")).is_err());
    }

    #[test]
    fn absolute_config_path_does_not_need_home() {
        assert_eq!(
            resolve_config_path_with_home("/tmp/config.toml", Path::new("/base"), None),
            Ok("/tmp/config.toml".to_string())
        );
        assert_eq!(watch_parent(Path::new("edgemap.toml")), Path::new("."));
    }

    #[test]
    fn tilde_config_path_requires_home() {
        assert!(resolve_config_path_with_home("~/config.toml", Path::new("/base"), None).is_err());
        assert_eq!(
            resolve_config_path_with_home("~/config.toml", Path::new("/base"), Some("/home/test")),
            Ok("/home/test/config.toml".to_string())
        );
    }

    fn profile(process: &str, cmdline: &str) -> ProfileConfig {
        ProfileConfig {
            config: "test.toml".to_string(),
            match_process: process.to_string(),
            match_cmdline: cmdline.to_string(),
        }
    }

    fn process(comm: Option<&str>, cmdline: Option<&str>) -> ProcessSnapshot {
        ProcessSnapshot {
            pid: 42,
            comm: comm.map(str::to_string),
            cmdline: cmdline.map(str::to_string),
        }
    }

    #[test]
    fn profile_match_requires_all_configured_fields() {
        let cfg = profile("game", "--profile edge");
        assert!(profile_matches(
            &process(Some("game"), Some("/usr/bin/game --profile edge")),
            &cfg
        ));
        assert!(!profile_matches(
            &process(Some("game"), Some("/usr/bin/game --profile default")),
            &cfg
        ));
        assert!(!profile_matches(
            &process(Some("launcher"), Some("/usr/bin/game --profile edge")),
            &cfg
        ));
    }

    #[test]
    fn profile_match_rejects_missing_process_data() {
        assert!(!profile_matches(&process(None, None), &profile("game", "")));
        assert!(!profile_matches(&process(None, None), &profile("", "game")));
    }

    #[test]
    fn empty_profile_does_not_match() {
        assert!(!profile_matches(
            &process(Some("game"), Some("game")),
            &profile("", "")
        ));
    }

    #[test]
    fn daemon_monitor_detects_config_write() {
        assert!(is_runtime_file(std::ffi::OsStr::new("control.sock")));
        assert!(!is_runtime_file(std::ffi::OsStr::new("connected")));
        assert!(!is_runtime_file(std::ffi::OsStr::new("needs-config")));
        assert!(!is_runtime_file(std::ffi::OsStr::new("unrelated")));
        assert!(needs_config_became_true(None, true));
        assert!(needs_config_became_true(Some(false), true));
        assert!(!needs_config_became_true(Some(true), true));
        assert!(!needs_config_became_true(Some(true), false));

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("edgemap-inotify-{}-{unique}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let config_path = dir.join("edgemap.toml");
        std::fs::write(&config_path, "config = \"default.toml\"\n").unwrap();

        let shutdown = ShutdownSignal::new().unwrap();
        let mut monitor = DaemonMonitor::new(&config_path).unwrap();
        assert_ne!(monitor.run_watch.is_some(), monitor.runtime_watch.is_some());
        std::fs::write(&config_path, "config = \"changed.toml\"\n").unwrap();
        let wake = monitor
            .wait(Instant::now() + Duration::from_secs(1), &shutdown, None)
            .unwrap();

        assert!(wake.config_changed);

        let result = unsafe { libc::pthread_kill(libc::pthread_self(), libc::SIGTERM) };
        assert_eq!(result, 0);
        let wake = monitor
            .wait(Instant::now() + Duration::from_secs(1), &shutdown, None)
            .unwrap();
        assert!(wake.shutdown);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn daemon_monitor_recovers_after_config_directory_recreation() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "edgemap-config-watch-{}-{unique}",
            std::process::id()
        ));
        let config_dir = root.join("edgemap");
        let config_path = config_dir.join("edgemap.toml");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(&config_path, "config = \"default.toml\"\n").unwrap();

        let shutdown = ShutdownSignal::new().unwrap();
        let mut monitor = DaemonMonitor::new(&config_path).unwrap();
        assert!(monitor.config_watch.is_some());
        assert!(monitor.config_parent_watch.is_none());

        std::fs::remove_file(&config_path).unwrap();
        std::fs::remove_dir(&config_dir).unwrap();
        let wake = monitor
            .wait(Instant::now() + Duration::from_secs(1), &shutdown, None)
            .unwrap();
        assert!(wake.config_changed);
        assert!(monitor.config_watch.is_none());
        assert!(monitor.config_parent_watch.is_some());

        std::fs::create_dir(&config_dir).unwrap();
        std::fs::write(&config_path, "config = \"restored.toml\"\n").unwrap();
        let wake = monitor
            .wait(Instant::now() + Duration::from_secs(1), &shutdown, None)
            .unwrap();
        assert!(wake.config_changed);
        assert!(monitor.config_watch.is_some());
        assert!(monitor.config_parent_watch.is_none());

        drop(monitor);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn daemon_monitor_fails_if_config_parent_disappears() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "edgemap-config-parent-{}-{unique}",
            std::process::id()
        ));
        let config_dir = root.join("edgemap");
        let config_path = config_dir.join("edgemap.toml");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(&config_path, "config = \"default.toml\"\n").unwrap();

        let shutdown = ShutdownSignal::new().unwrap();
        let mut monitor = DaemonMonitor::new(&config_path).unwrap();
        std::fs::remove_file(&config_path).unwrap();
        std::fs::remove_dir(&config_dir).unwrap();
        monitor
            .wait(Instant::now() + Duration::from_secs(1), &shutdown, None)
            .unwrap();
        assert!(monitor.config_parent_watch.is_some());

        std::fs::remove_dir(&root).unwrap();
        let error = match monitor.wait(Instant::now() + Duration::from_secs(1), &shutdown, None) {
            Err(error) => error,
            Ok(_) => panic!("config parent removal should fail the monitor"),
        };
        assert!(error.contains("config parent directory watch lost"));
    }

    #[test]
    fn child_reaper_waits_for_process_exit() {
        let child = std::process::Command::new("true").spawn().unwrap();
        reap_child(child).unwrap().join().unwrap();
    }
}
