use clap::Parser;
use std::process::ExitCode;

use sbox::cli::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();

    match sbox::app::run(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("sbox: {error}");
            ExitCode::from(1)
        }
    }
}
