use std::process::ExitCode;

use tracing::Level;

use crate::cli::{Cli, Commands};
use crate::error::SboxError;

pub fn run(cli: Cli) -> Result<ExitCode, SboxError> {
    init_logging(cli.verbose, cli.quiet)?;

    match &cli.command {
        Commands::Plan(command) => crate::plan::execute(&cli, command),
        Commands::Run(command) => crate::exec::execute_run(&cli, command),
        Commands::Exec(command) => crate::exec::execute_exec(&cli, command),
        Commands::Init(command) => crate::init::execute(&cli, command),
        Commands::Shell(command) => crate::shell::execute(&cli, command),
        Commands::Doctor(command) => crate::doctor::execute(&cli, command),
        Commands::Clean(command) => crate::clean::execute(&cli, command),
        Commands::Shim(command) => crate::shim::execute(command),
    }
}

fn init_logging(verbose: u8, quiet: bool) -> Result<(), SboxError> {
    let level = if quiet {
        Level::ERROR
    } else {
        match verbose {
            0 => Level::INFO,
            1 => Level::DEBUG,
            _ => Level::TRACE,
        }
    };

    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .without_time()
        .try_init()
        .map_err(|source| SboxError::LoggingInit { source })?;

    Ok(())
}
