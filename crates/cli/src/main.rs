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
        SubCommand::Check(config) => lint::run(&config),
        SubCommand::Fix(config) => fix::run_all(&config),
        SubCommand::Single(config) => fix::run_single(&config),
        SubCommand::Explain(config) => explain::run(&config),
        SubCommand::Dump(_) => dump::run(),
        SubCommand::List(_) => list::run(),
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
