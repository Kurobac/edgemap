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
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Deserialize;

const FIFO_PATH: &str = "/run/dseuhid/control";

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

fn edgemap_config_dir() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("edgemap");
        }
    }
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home).join(".config").join("edgemap");
    }
    PathBuf::from(".")
}

fn edgemap_state_dir() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_STATE_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("edgemap");
        }
    }
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home).join(".local").join("state").join("edgemap");
    }
    PathBuf::from(".")
}

fn resolve_config_path(raw: &str, base_dir: &Path) -> String {
    if raw.starts_with('/') {
        return raw.to_string();
    }
    if let Some(rest) = raw.strip_prefix('~') {
        if let Ok(home) = env::var("HOME") {
            return home + rest;
        }
        return raw.to_string();
    }
    base_dir.join(raw).to_string_lossy().into()
}

static DAEMON_RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn handle_daemon_signal(_sig: libc::c_int) {
    DAEMON_RUNNING.store(false, Ordering::SeqCst);
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

fn profile_matches(pid: u32, profile: &ProfileConfig) -> bool {
    if profile.match_process.is_empty() && profile.match_cmdline.is_empty() {
        return false;
    }
    if !profile.match_process.is_empty() {
        let comm = match read_comm(pid) {
            Some(c) => c,
            None => return false,
        };
        if comm != profile.match_process.to_lowercase() {
            return false;
        }
    }
    if !profile.match_cmdline.is_empty() {
        let cmdline = match read_cmdline(pid) {
            Some(c) => c,
            None => return false,
        };
        if !cmdline.contains(&profile.match_cmdline.to_lowercase()) {
            return false;
        }
    }
    true
}

fn find_matching_profile(profiles: &[(String, ProfileConfig)], config_dir: &Path, base_config: &str) -> Option<String> {
    let pids: Vec<u32> = match std::fs::read_dir("/proc") {
        Ok(d) => d.flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.parse().ok()))
            .collect(),
        Err(_) => return None,
    };
    for (profile_name, profile_cfg) in profiles {
        for &pid in &pids {
            if profile_matches(pid, profile_cfg) {
                log::debug!("profile '{}' matched by pid {pid}", profile_name);
                return Some(resolve_config_path(&profile_cfg.config, config_dir));
            }
        }
    }
    Some(resolve_config_path(base_config, config_dir))
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
    let dir = edgemap_config_dir();
    if !dir.exists() {
        println!("Config directory does not exist: {}", dir.display());
        std::process::exit(0);
    }
    let mut ok = 0;
    let mut fail = 0;
    let mut entries: Vec<_> = match std::fs::read_dir(&dir) {
        Ok(d) => d.flatten().filter(|e| {
            e.file_name().to_str().is_some_and(|n| n.ends_with(".toml") && n != "edgemap.toml")
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
    let path_str = if path.starts_with('.') {
        std::fs::canonicalize(path)
            .unwrap_or_else(|e| {
                eprintln!("error: cannot resolve {}: {}", path, e);
                std::process::exit(1);
            })
            .to_string_lossy()
            .to_string()
    } else {
        let config_dir = edgemap_config_dir();
        resolve_config_path(path, &config_dir)
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
        .unwrap_or("default.toml")
        .to_string();
    let base_config = resolve_config_path(&base_config_raw, &dir);

    // defer validation of base/default config to daemon loop (pre-injection)

    let mut profiles: Vec<(String, ProfileConfig)> = Vec::new();
    if let Some(t) = root.get("profiles").and_then(|v| v.as_table()) {
        for (name, val) in t.iter() {
            match val.clone().try_into::<ProfileConfig>() {
                Ok(cfg) => profiles.push((name.clone(), cfg)),
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
        let p_path = resolve_config_path(&pcfg.config, &dir);
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
    let mut edgemap_config_path = edgemap_config_dir().join("edgemap.toml");

    // parse optional --config <path> from args
    let mut i = 2;
    while i < args.len() {
        if args[i] == "--config" && i + 1 < args.len() {
            edgemap_config_path = PathBuf::from(resolve_config_path(&args[i + 1], &edgemap_config_dir()));
            i += 1;
        } else {
            eprintln!("error: unknown argument '{}'", args[i]);
            eprintln!("usage: edgemap daemon [--config <path>]");
            std::process::exit(1);
        }
        i += 1;
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let pid_path = edgemap_state_dir().join("edgemap.pid");
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
        let default_remap_path = dir.join("default.toml");
        if !default_remap_path.exists() {
            if let Err(e) = std::fs::write(&default_remap_path, default_toml_content) {
                log::error!("cannot write {}: {e}", default_remap_path.display());
                std::process::exit(1);
            }
            log::info!("Created {}", default_remap_path.display());
        }
        let edgemap_toml = "config = \"default.toml\"\n".to_string();
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

    let mut last_mtime = std::fs::metadata(&config_path)
        .and_then(|m| m.modified())
        .ok();

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

    let alive = check_dseuhid_alive();
    if !alive {
        log::warn!("dseuhid not running, waiting...");
    }

    let mut current_config = String::new();
    let mut last_pid: Option<i32> = None;
    let mut last_uhid_state: Option<String> = None;

    while DAEMON_RUNNING.load(Ordering::SeqCst) {
        // hot reload on mtime change
        if let Ok(meta) = std::fs::metadata(&config_path) {
            if let Ok(mtime) = meta.modified() {
                if last_mtime != Some(mtime) {
                    match load_edgemap_config(&config_path) {
                        Ok(s) => {
                            state = s;
                            log::info!("edgemap config reloaded");
                        }
                        Err(e) => log::error!("reload failed, keeping previous config: {e}"),
                    }
                    last_mtime = Some(mtime);
                }
            }
        }

        let alive = check_dseuhid_alive();
        if !alive {
            if last_uhid_state.as_deref() == Some("connected") {
                log::info!("UHID device stopped");
                send_notification("edgemap", "UHID device stopped");
            }
            if !current_config.is_empty() {
                log::warn!("dseuhid disconnected");
            }
            current_config.clear();
            last_pid = None;
            last_uhid_state = Some("disconnected".to_string());
            std::thread::sleep(Duration::from_secs(3));
            continue;
        }

        // detect dseuhid restart via PID change (systemctl restart is <1s)
        if let Ok(pid_str) = std::fs::read_to_string("/run/dseuhid/pid") {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                if last_pid != Some(pid) {
                    current_config.clear();
                    last_uhid_state = None;
                    last_pid = Some(pid);
                }
            }
        }

        // detect UHID virtual device state via /run/dseuhid/connected
        let uhid_state = std::fs::read_to_string("/run/dseuhid/connected")
            .unwrap_or_default()
            .trim()
            .to_string();

        // disconnected: skip injection entirely
        if uhid_state != "connected" {
            if last_uhid_state.as_deref() == Some("connected") {
                log::info!("UHID device stopped");
                send_notification("edgemap", "UHID device stopped");
            }
            last_uhid_state = Some(uhid_state);
            std::thread::sleep(Duration::from_secs(3));
            continue;
        }

        // connected: detect new UHID device via content transition
        // (only a "non-connected" → "connected" transition means the UHID
        // device was recreated and needs config re-injection)
        if last_uhid_state.as_deref() != Some("connected") {
            log::info!("UHID device ready");
            send_notification("edgemap", "UHID device ready");
            current_config.clear();
        }
        last_uhid_state = Some(uhid_state);

        let wanted = if state.valid_profiles.is_empty() {
            state.base_config.clone()
        } else {
            let valid: Vec<_> = state.profiles.iter()
                .filter(|(name, _)| state.valid_profiles.iter().any(|(vn, _)| vn == name))
                .cloned()
                .collect();
            find_matching_profile(&valid, &state.dir, &state.base_config_raw)
                .unwrap_or(state.base_config.clone())
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
                        std::thread::sleep(Duration::from_secs(3));
                        continue;
                    }
                } else {
                    // base_config itself is invalid — just warn, don't spam
                    log::warn!("default config invalid, keeping previous");
                    std::thread::sleep(Duration::from_secs(3));
                    continue;
                }
            }

            let cmd = format!("switch-config {}", target);
            if try_send_fifo_command(cmd.as_bytes()) {
                let label = state.profiles.iter()
                    .find(|(_, pc)| resolve_config_path(&pc.config, &state.dir) == target)
                    .map(|(name, _)| format!("profile '{name}'"))
                    .unwrap_or_else(|| "default config".to_string());
                log::info!("dseuhid connected");
                log::info!("applied {label}: {target}");
                send_notification("edgemap", &format!("Switched to {label}"));
                current_config = target;
            }
        }
        std::thread::sleep(Duration::from_secs(3));
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
