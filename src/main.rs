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

#[derive(Debug, PartialEq, Eq)]
enum CliAction {
    Run { config_path: Option<String> },
    Version,
    Help,
}

fn parse_cli(args: &[String]) -> Result<CliAction, String> {
    if let Some(command) = args.get(1) {
        let action = match command.as_str() {
            "version" | "--version" | "-V" => Some(CliAction::Version),
            "help" | "--help" | "-h" => Some(CliAction::Help),
            _ => None,
        };
        if let Some(action) = action {
            if args.len() > 2 {
                return Err(format!("command '{command}' does not accept arguments"));
            }
            return Ok(action);
        }
    }

    let mut config_path = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-c" | "--config-path" => {
                if config_path.is_some() {
                    return Err("option '--config-path' may only be specified once".into());
                }
                if i + 1 >= args.len() {
                    return Err("option '--config-path' requires a path".into());
                }
                config_path = Some(args[i + 1].clone());
                i += 2;
            }
            option if option.starts_with('-') => {
                return Err(format!("unknown option '{option}'"));
            }
            command if i == 1 => return Err(format!("unknown command '{command}'")),
            argument => return Err(format!("unexpected argument '{argument}'")),
        }
    }
    Ok(CliAction::Run { config_path })
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
    let action = match parse_cli(&args) {
        Ok(action) => action,
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!("hint: run 'dseuhid help' for usage");
            std::process::exit(1);
        }
    };
    let config_path = match action {
        CliAction::Version => {
            println!("dseuhid {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        CliAction::Help => {
            print_usage(true);
            return;
        }
        CliAction::Run { config_path } => config_path,
    };

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if unsafe { libc::getuid() } != 0 {
        error!("dseuhid daemon requires root");
        std::process::exit(1);
    }

    if let Err(e) =
        proxy::validate_repeat_env().and_then(|()| proxy::bt_haptics_buffer_from_env().map(|_| ()))
    {
        error!("{e}");
        std::process::exit(1);
    }

    if daemon::run(config_path) == daemon::DaemonExit::Fatal {
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

    #[test]
    fn cli_parser_covers_all_supported_aliases_and_invalid_shapes() {
        let parse = |arguments: &[&str]| {
            let args = std::iter::once("dseuhid".to_string())
                .chain(arguments.iter().map(|argument| (*argument).to_string()))
                .collect::<Vec<_>>();
            parse_cli(&args)
        };

        assert_eq!(parse(&[]), Ok(CliAction::Run { config_path: None }));
        for option in ["-c", "--config-path"] {
            assert_eq!(
                parse(&[option, "/tmp/config.toml"]),
                Ok(CliAction::Run {
                    config_path: Some("/tmp/config.toml".to_string()),
                })
            );
        }
        for command in ["help", "--help", "-h"] {
            assert_eq!(parse(&[command]), Ok(CliAction::Help));
        }
        for command in ["version", "--version", "-V"] {
            assert_eq!(parse(&[command]), Ok(CliAction::Version));
        }

        for arguments in [
            vec!["--unknown"],
            vec!["unknown"],
            vec!["help", "extra"],
            vec!["version", "extra"],
            vec!["-c"],
            vec!["--config-path"],
            vec!["-c", "one.toml", "--config-path", "two.toml"],
            vec!["-c", "one.toml", "extra"],
        ] {
            assert!(
                parse(&arguments).is_err(),
                "invalid arguments were accepted: {arguments:?}"
            );
        }
    }
}
