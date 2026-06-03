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
use std::path::Path;

const FIFO_PATH: &str = "/run/dseuhid/control";

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
