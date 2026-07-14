mod codec;
mod daemon;
mod descriptor;
mod device;
mod keyboard;
mod proxy;
mod session;
mod uhid;

use std::env;

use dseuhid::{config, control, keycodes, mapping, model, shutdown};
use log::error;

fn parse_config_path() -> Option<String> {
    let args: Vec<String> = env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-c" | "--config-path" => {
                if i + 1 >= args.len() {
                    eprintln!("error: option '--config-path' requires a path");
                    std::process::exit(1);
                }
                return Some(args[i + 1].clone());
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn usage_text() -> String {
    format!(
        concat!(
            "dseuhid {} — DualSense UHID proxy\n",
            "\n",
            "Usage: dseuhid [OPTIONS] [COMMAND]\n",
            "\n",
            "Commands:\n",
            "  version                   Print version and exit\n",
            "  help                      Print help\n",
            "\n",
            "Options:\n",
            "  -c, --config-path <PATH>  Load a config file; reconnect resets to passthrough\n",
            "\n",
            "Without a command, start the UHID proxy daemon (requires root).\n",
        ),
        env!("CARGO_PKG_VERSION")
    )
}

fn print_usage(to_stdout: bool) {
    let usage = usage_text();
    if to_stdout {
        print!("{usage}");
    } else {
        eprint!("{usage}");
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() >= 2 {
        let sub = args[1].as_str();
        let known = matches!(
            sub,
            "version" | "--version" | "-V" | "help" | "--help" | "-h"
        );
        if known && args.len() > 2 {
            eprintln!("error: command '{}' does not accept arguments", args[1]);
            eprintln!("hint: run 'dseuhid help' for usage");
            std::process::exit(1);
        }
        match sub {
            "version" | "--version" | "-V" => {
                println!("dseuhid {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "help" | "--help" | "-h" => {
                print_usage(true);
                return;
            }
            _ => {
                if !sub.starts_with('-') {
                    eprintln!("error: unknown command '{}'", args[1]);
                    eprintln!("hint: run 'dseuhid help' for usage");
                    std::process::exit(1);
                }
            }
        }
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if unsafe { libc::getuid() } != 0 {
        error!("dseuhid daemon requires root");
        std::process::exit(1);
    }

    if let Err(e) = proxy::validate_repeat_env() {
        error!("{e}");
        std::process::exit(1);
    }

    if daemon::run(parse_config_path()) == daemon::DaemonExit::Fatal {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod main_tests {
    use super::*;

    #[test]
    fn usage_uses_conventional_placeholders() {
        let usage = usage_text();
        assert!(usage.contains("Usage: dseuhid [OPTIONS] [COMMAND]"));
        assert!(usage.contains("--config-path <PATH>"));
        assert!(!usage.contains("<path>"));
    }
}
