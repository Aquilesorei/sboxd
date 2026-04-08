use clap::Parser;
use std::process::ExitCode;

use sbox::cli::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();

    sbox::app::run(cli).unwrap_or_else(|error| {
        eprintln!("sbox: {error}");
        ExitCode::from(1)
    })
}
