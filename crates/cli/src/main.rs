//! strictix command-line interface.
//!
//! Hand-written argument parsing (no clap). Commands: check, fix,
//! explain, list. Only `--help`/`--version` exist in M0.

use std::process::ExitCode;

const USAGE: &str = "\
strictix — strict lints and suggestions for the Nix language

USAGE:
    strictix <COMMAND> [ARGS]

COMMANDS:
    check     Lint files or directories
    fix       Lint and apply automatic fixes
    explain   Explain a lint rule by code
    list      List all lint rules

OPTIONS:
    -h, --help       Print this help
    -V, --version    Print version
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("-V" | "--version") => {
            println!("strictix {}", strictix_core::version());
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown command or option '{other}'\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
