use std::env;

mod cli;
mod control_session;
mod daemon;
mod paths;

use cli::{cmd_create_config, cmd_switch_config, cmd_validate, print_usage};

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
        "d" | "daemon" => daemon::cmd_daemon(&args),
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
