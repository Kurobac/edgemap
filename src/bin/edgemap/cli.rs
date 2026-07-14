use std::io::{self, Write};
use std::path::Path;

use dseuhid::{config, control};

use super::control_session::send_control_request;
use super::paths::{edgemap_config_dir, resolve_config_path, EDGEMAP_CONFIG_FILE};

pub(crate) const USAGE: &str = concat!(
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

pub(crate) fn print_usage(to_stdout: bool) {
    if to_stdout {
        print!("{USAGE}");
    } else {
        eprint!("{USAGE}");
    }
}

pub(crate) fn cmd_validate(args: &[String]) -> ! {
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

pub(crate) fn cmd_create_config(args: &[String]) -> ! {
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

pub(crate) fn cmd_switch_config(args: &[String]) -> ! {
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
