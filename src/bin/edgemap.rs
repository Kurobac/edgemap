#[path = "../report.rs"]
#[allow(dead_code)]
mod report;
#[path = "../mapping.rs"]
#[allow(dead_code)]
mod mapping;
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

#[derive(Debug, Deserialize)]
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
    eprintln!("  {:<28}Validate a config file", "v, validate <path>");
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

fn resolve_config_path(raw: &str, base_dir: &Path) -> String {
    if raw.starts_with('/') {
        return raw.to_string();
    }
    if raw.starts_with('~') {
        if let Ok(home) = env::var("HOME") {
            return home + &raw[1..];
        }
    }
    base_dir.join(raw).to_string_lossy().into()
}

static DAEMON_RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn handle_daemon_signal(_sig: libc::c_int) {
    DAEMON_RUNNING.store(false, Ordering::SeqCst);
}

fn send_notification(summary: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args([summary, body])
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
    if args.len() < 3 {
        eprintln!("error: validate requires a config path");
        eprintln!("usage: edgemap validate <path>");
        std::process::exit(1);
    }
    if args.len() > 3 {
        eprintln!("error: too many arguments");
        eprintln!("usage: edgemap validate <path>");
        std::process::exit(1);
    }
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
            println!("OK: {path} is valid");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
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
    let cfg = match config::Config::load(path) {
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
    let cmd = format!("switch-config {}", path);
    send_fifo_command(cmd.as_bytes())
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

    let edgemap_toml_content = match std::fs::read_to_string(&edgemap_config_path) {
        Ok(s) => s,
        Err(e) => {
            log::error!("cannot read {}: {e}", edgemap_config_path.display());
            std::process::exit(1);
        }
    };

    let root: toml::Value = match toml::from_str(&edgemap_toml_content) {
        Ok(v) => v,
        Err(e) => {
            log::error!("cannot parse {}: {e}", edgemap_config_path.display());
            std::process::exit(1);
        }
    };

    let base_config_raw = root.get("config")
        .and_then(|v| v.as_str())
        .unwrap_or("default.toml");
    let base_config = resolve_config_path(base_config_raw, dir);

    let mut profiles: Vec<(String, ProfileConfig)> = Vec::new();
    if let Some(t) = root.get("profiles").and_then(|v| v.as_table()) {
        for (name, val) in t.iter() {
            match val.clone().try_into::<ProfileConfig>() {
                Ok(cfg) => profiles.push((name.clone(), cfg)),
                Err(e) => log::warn!("invalid profile '{name}': {e}, skipping"),
            }
        }
    }

    // validate base config
    if !Path::new(&base_config).exists() {
        log::error!("config not found: {base_config}");
        log::error!("(specified in {})", edgemap_config_path.display());
        std::process::exit(1);
    }
    let base_cfg = match config::Config::load(&base_config) {
        Ok(c) => c,
        Err(e) => { log::error!("{e}"); std::process::exit(1); }
    };
    if let Err(e) = config::validate(&base_cfg) {
        log::error!("{e}");
        std::process::exit(1);
    }

    // validate and expand profile config paths, warn on invalid
    let mut valid_profiles: Vec<(String, String)> = Vec::new();
    for (name, pcfg) in &profiles {
        let path = resolve_config_path(&pcfg.config, dir);
        if !Path::new(&path).exists() {
            log::warn!("profile '{name}': config not found at {path}, skipping");
            continue;
        }
        match config::Config::load(&path) {
            Err(e) => { log::warn!("profile '{name}': {e}, skipping"); continue; }
            Ok(cfg) => {
                if let Err(e) = config::validate(&cfg) {
                    log::warn!("profile '{name}': {e}, skipping");
                    continue;
                }
            }
        }
        if pcfg.match_process.is_empty() && pcfg.match_cmdline.is_empty() {
            log::warn!("profile '{name}': no match_process or match_cmdline, skipping");
            continue;
        }
        valid_profiles.push((name.clone(), path));
    }
    if !profiles.is_empty() {
        log::info!("{} profile(s) loaded, {} valid", profiles.len(), valid_profiles.len());
    }

    // signal handlers
    unsafe {
        let handler = libc::SIG_DFL;
        let _ = libc::signal(libc::SIGPIPE, handler);
        libc::signal(libc::SIGINT, handle_daemon_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handle_daemon_signal as *const () as libc::sighandler_t);
    }

    log::info!("daemon started");
    log::info!("config: {}", edgemap_config_path.display());

    let alive = check_dseuhid_alive();
    if !alive {
        log::warn!("dseuhid not running, waiting...");
    }

    let mut current_config = String::new();

    while DAEMON_RUNNING.load(Ordering::SeqCst) {
        let alive = check_dseuhid_alive();
        if !alive {
            if !current_config.is_empty() {
                log::warn!("dseuhid disconnected");
                current_config.clear();
            }
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }

        let wanted = if valid_profiles.is_empty() {
            base_config.clone()
        } else {
            find_matching_profile(&profiles, dir, base_config_raw)
                .unwrap_or(base_config.clone())
        };

        if wanted != current_config {
            let cmd = format!("switch-config {}", wanted);
            if try_send_fifo_command(cmd.as_bytes()) {
                let label = profiles.iter()
                    .find(|(_, pc)| resolve_config_path(&pc.config, dir) == wanted)
                    .map(|(name, _)| format!("profile '{name}'"))
                    .unwrap_or_else(|| "default config".to_string());
                log::info!("applied {label}: {wanted}");
                send_notification("edgemap", &format!("Switched to {label}"));
                current_config = wanted;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }

    log::info!("daemon stopped");
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
