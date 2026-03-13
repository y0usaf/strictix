use clap::Parser;

#[rustfmt::skip]
use strictix::{
    config::{Opts, SubCommand},
    err::StatixErr,
    lint, fix, explain, dump, list,
};

fn main_() -> Result<(), StatixErr> {
    let opts = Opts::parse();
    match opts.cmd {
        SubCommand::Check(config) => lint::main::main(&config),
        SubCommand::Fix(config) => fix::main::all(&config),
        SubCommand::Single(config) => fix::main::single(&config),
        SubCommand::Explain(config) => explain::main::main(&config),
        SubCommand::Dump(_) => dump::main::main(),
        SubCommand::List(_) => list::main::main(),
    }
}

fn main() {
    match main_() {
        Ok(()) => {}
        Err(StatixErr::LintsFailed) => std::process::exit(1),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
