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
struct EdgemapConfig {
    config: String,
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

fn expand_tilde(path: &str) -> String {
    if path.starts_with('~') {
        if let Ok(home) = env::var("HOME") {
            return home + &path[1..];
        }
    }
    path.to_string()
}

static DAEMON_RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn handle_daemon_signal(_sig: libc::c_int) {
    DAEMON_RUNNING.store(false, Ordering::SeqCst);
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
            edgemap_config_path = PathBuf::from(expand_tilde(&args[i + 1]));
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
        let edgemap_toml = format!("config = \"{}\"\n", default_remap_path.display());
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
    let edgemap_cfg: EdgemapConfig = match toml::from_str(&edgemap_toml_content) {
        Ok(c) => c,
        Err(e) => {
            log::error!("cannot parse {}: {e}", edgemap_config_path.display());
            std::process::exit(1);
        }
    };
    let remap_path = expand_tilde(&edgemap_cfg.config);

    if !Path::new(&remap_path).exists() {
        log::error!("config not found: {remap_path}");
        log::error!("(specified in {})", edgemap_config_path.display());
        std::process::exit(1);
    }
    let remap_cfg = match config::Config::load(&remap_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("{e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = config::validate(&remap_cfg) {
        log::error!("{e}");
        std::process::exit(1);
    }

    let cmd = format!("switch-config {}", remap_path);

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

    let mut injected = false;
    while DAEMON_RUNNING.load(Ordering::SeqCst) {
        let alive = check_dseuhid_alive();
        if alive && !injected {
            if try_send_fifo_command(cmd.as_bytes()) {
                log::info!("dseuhid connected, applied: {remap_path}");
                injected = true;
            }
        } else if !alive && injected {
            log::warn!("dseuhid disconnected");
            injected = false;
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
