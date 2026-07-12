#[path = "../report.rs"]
#[allow(dead_code)]
mod report;
#[path = "../mapping.rs"]
#[allow(dead_code)]
mod mapping;
#[path = "../keyboard.rs"]
#[allow(dead_code)]
mod keyboard;
#[path = "../config.rs"]
#[allow(dead_code)]
mod config;

use std::env;
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, WatchDescriptor};
use serde::Deserialize;

const FIFO_PATH: &str = "/run/dseuhid/control";
const DSEUHID_RUNTIME_DIR: &str = "/run/dseuhid";
const CONNECTED_PATH: &str = "/run/dseuhid/connected";
const NEEDS_CONFIG_PATH: &str = "/run/dseuhid/needs-config";
const RUN_DIR: &str = "/run";
const CONTROL_FILE_NAME: &str = "control";
const CONNECTED_FILE_NAME: &str = "connected";
const NEEDS_CONFIG_FILE_NAME: &str = "needs-config";
const STATE_CONNECTED: &str = "connected";
const STATE_DISCONNECTED: &str = "disconnected";
const EDGEMAP_CONFIG_FILE: &str = "edgemap.toml";
const DEFAULT_CONFIG_FILE: &str = "default.toml";
const PROFILE_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Deserialize)]
struct ProfileConfig {
    config: String,
    #[serde(default)]
    match_process: String,
    #[serde(default)]
    match_cmdline: String,
}

fn print_usage() {
    eprintln!("edgemap — companion CLI for dseuhid");
    eprintln!();
    eprintln!("Usage: edgemap <command> [args]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  {:<28}Validate config file(s)", "v, validate [path]");
    eprintln!("  {:<28}Create default config (stdout if no path)", "cc, create-config [path]");
    eprintln!("  {:<28}Tell running daemon to reload config", "r, reload");
    eprintln!("  {:<28}Tell daemon to load a different config", "sc, switch-config <path>");
    eprintln!("  {:<28}Watch daemon and inject config (auto-start)", "d, daemon [--config <path>]");
}

fn required_home() -> Result<String, String> {
    match env::var("HOME") {
        Ok(home) if !home.is_empty() => Ok(home),
        Ok(_) | Err(env::VarError::NotPresent) => Err("HOME is not set or is empty".to_string()),
        Err(env::VarError::NotUnicode(_)) => Err("HOME is not valid Unicode".to_string()),
    }
}

fn resolve_xdg_dir(
    xdg: Option<&Path>,
    home: Option<&str>,
    fallback: &Path,
) -> Result<PathBuf, String> {
    if let Some(xdg) = xdg {
        if !xdg.as_os_str().is_empty() && xdg.is_absolute() {
            return Ok(xdg.join("edgemap"));
        }
    }
    let home = home.ok_or_else(|| "HOME is not set or is empty".to_string())?;
    Ok(PathBuf::from(home).join(fallback).join("edgemap"))
}

fn xdg_dir(var: &str, fallback: &Path) -> Result<PathBuf, String> {
    let xdg = env::var_os(var).map(PathBuf::from);
    if xdg.as_deref().is_some_and(|path| {
        !path.as_os_str().is_empty() && path.is_absolute()
    }) {
        return resolve_xdg_dir(xdg.as_deref(), None, fallback);
    }
    let home = required_home()?;
    resolve_xdg_dir(xdg.as_deref(), Some(&home), fallback)
}

fn edgemap_config_dir() -> Result<PathBuf, String> {
    xdg_dir("XDG_CONFIG_HOME", Path::new(".config"))
}

fn edgemap_state_dir() -> Result<PathBuf, String> {
    xdg_dir("XDG_STATE_HOME", Path::new(".local/state"))
}

fn resolve_config_path_with_home(
    raw: &str,
    base_dir: &Path,
    home: Option<&str>,
) -> Result<String, String> {
    if raw.starts_with('/') {
        return Ok(raw.to_string());
    }
    if let Some(rest) = raw.strip_prefix('~') {
        let home = home.ok_or_else(|| "HOME is not set or is empty".to_string())?;
        return Ok(home.to_string() + rest);
    }
    Ok(base_dir.join(raw).to_string_lossy().into())
}

fn resolve_config_path(raw: &str, base_dir: &Path) -> Result<String, String> {
    let home = if raw.starts_with('~') {
        Some(required_home()?)
    } else {
        None
    };
    resolve_config_path_with_home(raw, base_dir, home.as_deref())
}

static DAEMON_RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn handle_daemon_signal(_sig: libc::c_int) {
    DAEMON_RUNNING.store(false, Ordering::SeqCst);
}

#[derive(Default)]
struct DaemonWake {
    config_changed: bool,
    runtime_changed: bool,
    profile_due: bool,
}

struct DaemonMonitor {
    inotify: Inotify,
    config_watch: WatchDescriptor,
    run_watch: Option<WatchDescriptor>,
    runtime_watch: Option<WatchDescriptor>,
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

fn watch_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn is_runtime_file(name: &std::ffi::OsStr) -> bool {
    name == CONTROL_FILE_NAME
        || name == CONNECTED_FILE_NAME
        || name == NEEDS_CONFIG_FILE_NAME
}

fn parse_needs_config(content: &str) -> Result<bool, String> {
    match content.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        value => Err(format!("invalid needs-config state: {value:?}")),
    }
}

fn read_needs_config() -> Result<bool, String> {
    let content = std::fs::read_to_string(NEEDS_CONFIG_PATH)
        .map_err(|e| format!("cannot read {NEEDS_CONFIG_PATH}: {e}"))?;
    parse_needs_config(&content)
}

fn needs_config_became_true(previous: Option<bool>, current: bool) -> bool {
    current && previous != Some(true)
}

impl DaemonMonitor {
    fn new(config_path: &Path) -> Result<Self, String> {
        let inotify = Inotify::init(InitFlags::IN_CLOEXEC | InitFlags::IN_NONBLOCK)
            .map_err(|e| format!("cannot initialize inotify: {e}"))?;
        let watch_flags = daemon_watch_flags();
        let config_dir = watch_parent(config_path);
        let config_watch = inotify
            .add_watch(config_dir, watch_flags)
            .map_err(|e| format!("cannot watch {}: {e}", config_dir.display()))?;
        let runtime_exists = Path::new(DSEUHID_RUNTIME_DIR).is_dir();
        let runtime_watch = if runtime_exists {
            Some(
                inotify
                    .add_watch(DSEUHID_RUNTIME_DIR, watch_flags)
                    .map_err(|e| format!("cannot watch {DSEUHID_RUNTIME_DIR}: {e}"))?,
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
                    .map_err(|e| format!("cannot watch {RUN_DIR}: {e}"))?,
            )
        };
        let config_name = config_path
            .file_name()
            .ok_or_else(|| format!("invalid config path: {}", config_path.display()))?
            .to_os_string();
        Ok(Self {
            inotify,
            config_watch,
            run_watch,
            runtime_watch,
            config_name,
        })
    }

    fn ensure_runtime_watch(&mut self) -> Result<(), String> {
        if self.runtime_watch.is_none() && Path::new(DSEUHID_RUNTIME_DIR).is_dir() {
            self.runtime_watch = Some(
                self.inotify
                    .add_watch(DSEUHID_RUNTIME_DIR, daemon_watch_flags())
                    .map_err(|e| format!("cannot watch {DSEUHID_RUNTIME_DIR}: {e}"))?,
            );
            if let Some(run_watch) = self.run_watch.take() {
                self.inotify
                    .rm_watch(run_watch)
                    .map_err(|e| format!("cannot stop watching {RUN_DIR}: {e}"))?;
            }
        }
        Ok(())
    }

    fn ensure_run_watch(&mut self) -> Result<(), String> {
        if self.runtime_watch.is_none() && self.run_watch.is_none() {
            self.run_watch = Some(
                self.inotify
                    .add_watch(RUN_DIR, run_discovery_flags())
                    .map_err(|e| format!("cannot watch {RUN_DIR}: {e}"))?,
            );
        }
        Ok(())
    }

    fn wait(&mut self, deadline: Instant) -> Result<DaemonWake, String> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as u32;
        let mut fds = [PollFd::new(self.inotify.as_fd(), PollFlags::POLLIN)];
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
        let events = self
            .inotify
            .read_events()
            .map_err(|e| format!("cannot read inotify events: {e}"))?;
        for event in events {
            if event.mask.contains(AddWatchFlags::IN_Q_OVERFLOW) {
                return Err("inotify event queue overflowed".to_string());
            }
            if event.wd == self.config_watch
                && event.name.as_deref() == Some(self.config_name.as_os_str())
            {
                wake.config_changed = true;
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
        self.ensure_runtime_watch()?;
        self.ensure_run_watch()?;
        wake.profile_due = Instant::now() >= deadline;
        Ok(wake)
    }
}

fn wait_for_daemon_activity(
    monitor: &mut DaemonMonitor,
    next_profile_scan: &mut Instant,
    config_changed: &mut bool,
    runtime_changed: &mut bool,
    profile_due: &mut bool,
) -> Result<(), String> {
    let wake = monitor.wait(*next_profile_scan)?;
    *config_changed |= wake.config_changed;
    *runtime_changed |= wake.runtime_changed;
    if wake.profile_due {
        *profile_due = true;
        *next_profile_scan = Instant::now() + PROFILE_INTERVAL;
    }
    Ok(())
}

fn send_notification(summary: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "edgemap", summary, body])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn check_dseuhid_alive() -> bool {
    let path = Path::new(FIFO_PATH);
    match path.metadata() {
        Ok(meta) => {
            if !meta.file_type().is_fifo() {
                return false;
            }
        }
        Err(_) => return false,
    }
    std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(FIFO_PATH)
        .is_ok()
}

fn try_send_fifo_command(cmd: &[u8]) -> bool {
    let path = Path::new(FIFO_PATH);
    match path.metadata() {
        Ok(meta) => {
            if !meta.file_type().is_fifo() {
                return false;
            }
        }
        Err(_) => {
            return false;
        }
    }
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(FIFO_PATH)
    {
        Ok(f) => f,
        Err(_) => {
            return false;
        }
    };
    if file.write_all(cmd).is_err() {
        return false;
    }
    if file.write_all(b"\n").is_err() {
        return false;
    }
    true
}

fn read_comm(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|s| s.trim().to_lowercase())
}

fn read_cmdline(pid: u32) -> Option<String> {
    let data = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if data.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&data).replace('\0', " ").to_lowercase())
}

#[derive(Debug, Clone)]
struct ProcessSnapshot {
    pid: u32,
    comm: Option<String>,
    cmdline: Option<String>,
}

fn profile_matches(process: &ProcessSnapshot, profile: &ProfileConfig) -> bool {
    // load_edgemap_config() lowercases profile match fields; process snapshots
    // are lowercased when read from /proc, so comparisons here stay allocation-free.
    if profile.match_process.is_empty() && profile.match_cmdline.is_empty() {
        return false;
    }
    if !profile.match_process.is_empty() {
        let comm = match process.comm.as_deref() {
            Some(comm) => comm,
            None => return false,
        };
        if comm != profile.match_process {
            return false;
        }
    }
    if !profile.match_cmdline.is_empty() {
        let cmdline = match process.cmdline.as_deref() {
            Some(cmdline) => cmdline,
            None => return false,
        };
        if !cmdline.contains(&profile.match_cmdline) {
            return false;
        }
    }
    true
}

fn snapshot_processes(profiles: &[(String, ProfileConfig)]) -> Vec<ProcessSnapshot> {
    let need_comm = profiles
        .iter()
        .any(|(_, profile)| !profile.match_process.is_empty());
    let need_cmdline = profiles
        .iter()
        .any(|(_, profile)| !profile.match_cmdline.is_empty());

    let entries = match std::fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse().ok()?;
            Some(ProcessSnapshot {
                pid,
                comm: need_comm.then(|| read_comm(pid)).flatten(),
                cmdline: need_cmdline.then(|| read_cmdline(pid)).flatten(),
            })
        })
        .collect()
}

fn find_matching_profile(
    profiles: &[(String, ProfileConfig)],
    config_dir: &Path,
    base_config: &str,
) -> Result<Option<String>, String> {
    let processes = snapshot_processes(profiles);
    for (profile_name, profile_cfg) in profiles {
        for process in &processes {
            if profile_matches(process, profile_cfg) {
                log::debug!("profile '{}' matched by pid {}", profile_name, process.pid);
                return resolve_config_path(&profile_cfg.config, config_dir).map(Some);
            }
        }
    }
    resolve_config_path(base_config, config_dir).map(Some)
}

fn cmd_validate(args: &[String]) -> ! {
    if args.len() > 3 {
        eprintln!("error: too many arguments");
        eprintln!("usage: edgemap validate [path]");
        std::process::exit(1);
    }

    if args.len() == 3 {
        let path = &args[2];
        let cfg = match config::Config::load(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        };
        match config::validate(&cfg) {
            Ok(()) => {
                if cfg.buttons.is_empty() {
                    println!("OK: {path} is valid (passthrough only)");
                } else {
                    println!("OK: {path} is valid");
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
        println!("Config directory does not exist: {}", dir.display());
        std::process::exit(0);
    }
    let mut ok = 0;
    let mut fail = 0;
    let mut entries: Vec<_> = match std::fs::read_dir(&dir) {
        Ok(d) => d.flatten().filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.ends_with(".toml") && n != EDGEMAP_CONFIG_FILE)
        }).collect(),
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", dir.display());
            std::process::exit(1);
        }
    };
    entries.sort_by_key(|e| e.file_name());

    println!("Checking {} ...", dir.display());
    for entry in entries {
        let path = entry.path();
        let display = entry.file_name().to_string_lossy().into_owned();
        match config::Config::load(path.to_str().unwrap()) {
            Ok(cfg) => match config::validate(&cfg) {
                Ok(()) => {
                    let note = if cfg.buttons.is_empty() { " (passthrough only)" } else { "" };
                    println!("  {display} ... OK{note}");
                    ok += 1;
                }
                Err(e) => { eprintln!("  {display} ... FAIL: {e}"); fail += 1; }
            },
            Err(e) => { eprintln!("  {display} ... FAIL: {e}"); fail += 1; }
        }
    }
    let total = ok + fail;
    println!("{ok}/{total} valid");
    std::process::exit(if fail > 0 { 1 } else { 0 });
}

fn cmd_create_config(args: &[String]) -> ! {
    if args.len() > 3 {
        eprintln!("error: too many arguments");
        eprintln!("usage: edgemap create-config [path]");
        std::process::exit(1);
    }
    let content = config::default_content();
    if args.len() >= 3 {
        let path = &args[2];
        if Path::new(path).exists() {
            eprintln!("error: {path} already exists");
            std::process::exit(1);
        }
        if let Some(parent) = Path::new(path).parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("error: cannot create parent dir for {path}: {e}");
                std::process::exit(1);
            }
        }
        if let Err(e) = std::fs::write(path, content) {
            eprintln!("error: cannot write {path}: {e}");
            std::process::exit(1);
        }
        println!("Created {path}");
    } else {
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        if let Err(e) = handle.write_all(content.as_bytes()) {
            eprintln!("error: cannot write to stdout: {e}");
            std::process::exit(1);
        }
    }
    std::process::exit(0);
}

fn send_fifo_command(cmd: &[u8]) -> ! {
    let path = Path::new(FIFO_PATH);
    match path.metadata() {
        Ok(meta) => {
            if !meta.file_type().is_fifo() {
                eprintln!("error: {} is not a FIFO (is dseuhid running?)", FIFO_PATH);
                std::process::exit(1);
            }
        }
        Err(_) => {
            eprintln!("error: {} does not exist (is dseuhid running?)", FIFO_PATH);
            std::process::exit(1);
        }
    }

    let file = match std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(FIFO_PATH)
    {
        Ok(f) => f,
        Err(e) => {
            let errno = e.raw_os_error();
            if errno == Some(libc::ENXIO) {
                eprintln!("error: no reader on {} (is dseuhid running?)", FIFO_PATH);
            } else {
                eprintln!("error: cannot open {}: {e}", FIFO_PATH);
            }
            std::process::exit(1);
        }
    };

    let mut file = file;
    if let Err(e) = file.write_all(cmd) {
        eprintln!("error: cannot write to {}: {e}", FIFO_PATH);
        std::process::exit(1);
    }
    if let Err(e) = file.write_all(b"\n") {
        eprintln!("error: cannot write to {}: {e}", FIFO_PATH);
        std::process::exit(1);
    }
    eprintln!("Command sent to dseuhid");
    std::process::exit(0);
}

fn cmd_reload(args: &[String]) -> ! {
    if args.len() > 2 {
        eprintln!("error: reload takes no arguments");
        eprintln!("usage: edgemap reload");
        std::process::exit(1);
    }
    send_fifo_command(b"reload")
}

fn cmd_switch_config(args: &[String]) -> ! {
    if args.len() < 3 {
        eprintln!("error: switch-config requires a path argument");
        eprintln!("usage: edgemap switch-config <path>");
        std::process::exit(1);
    }
    if args.len() > 3 {
        eprintln!("error: too many arguments");
        eprintln!("usage: edgemap switch-config <path>");
        std::process::exit(1);
    }
    let path = &args[2];
    let path_str = if Path::new(path).is_absolute() {
        path.clone()
    } else if path.starts_with('.') {
        std::fs::canonicalize(path)
            .unwrap_or_else(|e| {
                eprintln!("error: cannot resolve {}: {}", path, e);
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
    let cfg = match config::Config::load(&path_str) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = config::validate(&cfg) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
    let cmd = format!("switch-config {}", path_str);
    send_fifo_command(cmd.as_bytes())
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
            trimmed.strip_prefix("[profiles.")
                .and_then(|rest| rest.strip_suffix(']'))
                .map(|s| s.to_string())
        })
        .collect()
}

fn load_edgemap_config(path: &Path) -> Result<DaemonState, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let root: toml::Value = toml::from_str(&content)
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let base_config_raw = root.get("config")
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
                Err(e) => log::warn!("invalid profile '{name}': {e}, skipping"),
            }
        }
    }

    // sort by declaration order in the TOML file
    let decl_order = extract_profile_order(&content);
    profiles.sort_by_key(|(name, _)| {
        decl_order.iter().position(|n| n == name).unwrap_or(usize::MAX)
    });

    let mut valid_profiles: Vec<(String, String)> = Vec::new();
    for (name, pcfg) in &profiles {
        let p_path = resolve_config_path(&pcfg.config, &dir)?;
        if pcfg.match_process.is_empty() && pcfg.match_cmdline.is_empty() {
            log::warn!("profile '{name}': no match_process or match_cmdline, skipping");
            continue;
        }
        // defer config existence/validation to daemon loop (pre-injection)
        valid_profiles.push((name.clone(), p_path));
    }

    if !profiles.is_empty() {
        log::info!("{} profile(s) loaded, {} valid", profiles.len(), valid_profiles.len());
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
            eprintln!("usage: edgemap daemon [--config <path>]");
            std::process::exit(1);
        }
        i += 1;
    }

    let edgemap_config_path = match config_arg {
        Some(path) if Path::new(path).is_absolute() => Ok(PathBuf::from(path)),
        Some(path) if path.starts_with('~') => resolve_config_path(path, Path::new(""))
            .map(PathBuf::from),
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

    let pid_path = edgemap_state_dir()
        .unwrap_or_else(|e| {
            log::error!("{e}");
            std::process::exit(1);
        })
        .join("edgemap.pid");
    if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            if unsafe { libc::kill(pid, 0) } == 0 {
                log::error!("another edgemap daemon is already running (PID {pid})");
                std::process::exit(1);
            }
        }
    }

    let dir = edgemap_config_path.parent().unwrap_or(Path::new("."));

    if !edgemap_config_path.exists() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::error!("cannot create {}: {e}", dir.display());
            std::process::exit(1);
        }
        let default_toml_content = config::default_content();
        let default_remap_path = dir.join(DEFAULT_CONFIG_FILE);
        if !default_remap_path.exists() {
            if let Err(e) = std::fs::write(&default_remap_path, default_toml_content) {
                log::error!("cannot write {}: {e}", default_remap_path.display());
                std::process::exit(1);
            }
            log::info!("Created {}", default_remap_path.display());
        }
        let edgemap_toml = format!("config = \"{DEFAULT_CONFIG_FILE}\"\n");
        if let Err(e) = std::fs::write(&edgemap_config_path, edgemap_toml) {
            log::error!("cannot write {}: {e}", edgemap_config_path.display());
            std::process::exit(1);
        }
        log::info!("Created {}", edgemap_config_path.display());
    }

    let config_path = edgemap_config_path.clone();
    let mut state = match load_edgemap_config(&config_path) {
        Ok(s) => s,
        Err(e) => {
            log::error!("{e}");
            std::process::exit(1);
        }
    };

    // signal handlers
    unsafe {
        let handler = libc::SIG_DFL;
        let _ = libc::signal(libc::SIGPIPE, handler);
        libc::signal(libc::SIGINT, handle_daemon_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handle_daemon_signal as *const () as libc::sighandler_t);
    }

    log::info!("daemon started");
    log::info!("config: {}", config_path.display());
    if let Some(parent) = pid_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&pid_path, std::process::id().to_string()).ok();

    let mut monitor = DaemonMonitor::new(&config_path).unwrap_or_else(|e| {
        log::error!("{e}");
        std::process::exit(1);
    });

    let mut daemon_alive = check_dseuhid_alive();
    if !daemon_alive {
        log::warn!("dseuhid not running, waiting...");
    }

    let mut current_config = String::new();
    let mut last_uhid_state: Option<String> = None;
    let mut last_needs_config: Option<bool> = None;
    let mut config_changed = false;
    let mut runtime_changed = true;
    let mut profile_due = true;
    let mut next_profile_scan = Instant::now() + PROFILE_INTERVAL;

    while DAEMON_RUNNING.load(Ordering::SeqCst) {
        if config_changed {
            config_changed = false;
            match load_edgemap_config(&config_path) {
                Ok(s) => {
                    state = s;
                    current_config.clear();
                    profile_due = true;
                    log::info!("edgemap config reloaded");
                }
                Err(e) => log::error!("reload failed, keeping previous config: {e}"),
            }
        }

        if runtime_changed {
            runtime_changed = false;
            let was_daemon_alive = daemon_alive;
            daemon_alive = check_dseuhid_alive();
            if !daemon_alive {
                if last_uhid_state.as_deref() == Some(STATE_CONNECTED) {
                    log::info!("UHID device stopped");
                }
                if was_daemon_alive && !current_config.is_empty() {
                    log::warn!("dseuhid disconnected");
                }
                last_needs_config = None;
                last_uhid_state = Some(STATE_DISCONNECTED.to_string());
            } else {
                match read_needs_config() {
                    Ok(needs_config) => {
                        if needs_config_became_true(last_needs_config, needs_config) {
                            current_config.clear();
                            profile_due = true;
                        }
                        last_needs_config = Some(needs_config);
                    }
                    Err(e) => log::error!("{e}"),
                }

                // detect UHID virtual device state via /run/dseuhid/connected
                let uhid_state = std::fs::read_to_string(CONNECTED_PATH)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if uhid_state != STATE_CONNECTED {
                    if last_uhid_state.as_deref() == Some(STATE_CONNECTED) {
                        log::info!("UHID device stopped");
                    }
                } else if last_uhid_state.as_deref() != Some(STATE_CONNECTED) {
                    log::info!("UHID device ready");
                }
                last_uhid_state = Some(uhid_state);
            }
        }

        // A disconnected daemon or UHID device waits for an explicit runtime event.
        if !daemon_alive || last_uhid_state.as_deref() != Some(STATE_CONNECTED) {
            if let Err(e) = wait_for_daemon_activity(
                &mut monitor,
                &mut next_profile_scan,
                &mut config_changed,
                &mut runtime_changed,
                &mut profile_due,
            ) {
                log::error!("{e}");
                break;
            }
            continue;
        }

        if !profile_due {
            if let Err(e) = wait_for_daemon_activity(
                &mut monitor,
                &mut next_profile_scan,
                &mut config_changed,
                &mut runtime_changed,
                &mut profile_due,
            ) {
                log::error!("{e}");
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
                    log::error!("cannot resolve profile config: {e}");
                    if let Err(wait_error) = wait_for_daemon_activity(
                        &mut monitor,
                        &mut next_profile_scan,
                        &mut config_changed,
                        &mut runtime_changed,
                        &mut profile_due,
                    ) {
                        log::error!("{wait_error}");
                        break;
                    }
                    continue;
                }
            }
        };

        if wanted != current_config {
            // validate before injecting — catches profiles configured before
            // their config files are created, or invalid save states
            let is_valid = |p: &str| -> bool {
                if !Path::new(p).exists() {
                    log::warn!("config not found: {p}");
                    return false;
                }
                match config::Config::load(p) {
                    Ok(cfg) => {
                        if let Err(e) = config::validate(&cfg) {
                            log::warn!("config invalid ({p}): {e}");
                            false
                        } else { true }
                    }
                    Err(e) => { log::warn!("cannot load {p}: {e}"); false }
                }
            };

            let mut target = wanted.clone();
            if !is_valid(&target) {
                // profile config failed — try base_config as fallback
                if target != state.base_config {
                    log::warn!("profile config invalid, falling back to default");
                    target = state.base_config.clone();
                    if !is_valid(&target) {
                        log::warn!("default config also invalid, keeping previous");
                        if let Err(wait_error) = wait_for_daemon_activity(
                            &mut monitor,
                            &mut next_profile_scan,
                            &mut config_changed,
                            &mut runtime_changed,
                            &mut profile_due,
                        ) {
                            log::error!("{wait_error}");
                            break;
                        }
                        continue;
                    }
                } else {
                    // base_config itself is invalid — just warn, don't spam
                    log::warn!("default config invalid, keeping previous");
                    if let Err(wait_error) = wait_for_daemon_activity(
                        &mut monitor,
                        &mut next_profile_scan,
                        &mut config_changed,
                        &mut runtime_changed,
                        &mut profile_due,
                    ) {
                        log::error!("{wait_error}");
                        break;
                    }
                    continue;
                }
            }

            let cmd = format!("switch-config {}", target);
            if try_send_fifo_command(cmd.as_bytes()) {
                let label = state.profiles.iter()
                    .find(|(_, pc)| {
                        resolve_config_path(&pc.config, &state.dir).as_deref()
                            == Ok(target.as_str())
                    })
                    .map(|(name, _)| format!("profile '{name}'"))
                    .unwrap_or_else(|| "default config".to_string());
                log::info!("applied {label}: {target}");
                send_notification("edgemap", &format!("Switched to {label}"));
                current_config = target;
            }
        }
        if let Err(e) = wait_for_daemon_activity(
            &mut monitor,
            &mut next_profile_scan,
            &mut config_changed,
            &mut runtime_changed,
            &mut profile_due,
        ) {
            log::error!("{e}");
            break;
        }
    }

    log::info!("daemon stopped");
    let _ = std::fs::remove_file(&pid_path);
    std::process::exit(0);
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }
    match args[1].as_str() {
        "v" | "validate" => cmd_validate(&args),
        "cc" | "create-config" => cmd_create_config(&args),
        "r" | "reload" => cmd_reload(&args),
        "sc" | "switch-config" => cmd_switch_config(&args),
        "d" | "daemon" => cmd_daemon(&args),
        "help" | "--help" | "-h" => {
            print_usage();
            std::process::exit(0);
        }
        _ => {
            eprintln!("error: unknown command '{}'", args[1]);
            eprintln!("Run 'edgemap help' for usage.");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

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
        assert!(resolve_config_path_with_home("~/config.toml", Path::new("/base"), None)
            .is_err());
        assert_eq!(
            resolve_config_path_with_home(
                "~/config.toml",
                Path::new("/base"),
                Some("/home/test")
            ),
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
        for name in ["control", "connected", "needs-config"] {
            assert!(is_runtime_file(std::ffi::OsStr::new(name)));
        }
        assert!(!is_runtime_file(std::ffi::OsStr::new("unrelated")));
        assert_eq!(parse_needs_config("true\n"), Ok(true));
        assert_eq!(parse_needs_config("false\n"), Ok(false));
        assert!(parse_needs_config("").is_err());
        assert!(parse_needs_config("yes").is_err());
        assert!(needs_config_became_true(None, true));
        assert!(needs_config_became_true(Some(false), true));
        assert!(!needs_config_became_true(Some(true), true));
        assert!(!needs_config_became_true(Some(true), false));

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("edgemap-inotify-{}-{unique}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let config_path = dir.join("edgemap.toml");
        std::fs::write(&config_path, "config = \"default.toml\"\n").unwrap();

        let mut monitor = DaemonMonitor::new(&config_path).unwrap();
        assert_ne!(monitor.run_watch.is_some(), monitor.runtime_watch.is_some());
        std::fs::write(&config_path, "config = \"changed.toml\"\n").unwrap();
        let wake = monitor.wait(Instant::now() + Duration::from_secs(1)).unwrap();

        assert!(wake.config_changed);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
