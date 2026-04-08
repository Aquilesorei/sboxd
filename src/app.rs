use std::process::ExitCode;

use tracing::Level;

use crate::cli::{Cli, Commands};
use crate::error::SboxError;

pub fn run(cli: Cli) -> Result<ExitCode, SboxError> {
    init_logging(cli.verbose, cli.quiet)?;

    match &cli.command {
        Some(Commands::Plan(command)) => crate::plan::execute(&cli, command),
        Some(Commands::Run(command)) => crate::exec::execute_run(&cli, command),
        Some(Commands::Exec(command)) => crate::exec::execute_exec(&cli, command),
        Some(Commands::Init(command)) => crate::init::execute(&cli, command),
        Some(Commands::Shell(command)) => crate::shell::execute(&cli, command),
        Some(Commands::Doctor(command)) => crate::doctor::execute(&cli, command),
        Some(Commands::Clean(command)) => crate::clean::execute(&cli, command),
        Some(Commands::Shim(command)) => crate::shim::execute(command),
        Some(Commands::Bootstrap(_)) => crate::bootstrap::execute(&cli),
        Some(Commands::Audit(command)) => crate::audit::execute(&cli, command),
        Some(Commands::Harden(command)) => crate::harden::execute(&cli, command),
        Some(Commands::Completions(command)) => {
            crate::cli::generate_completions(command.shell);
            Ok(std::process::ExitCode::SUCCESS)
        }
        Some(Commands::Status(command)) => crate::status::execute(&cli, command),
        Some(Commands::Logs(command)) => crate::logs::execute(&cli, command),
        Some(Commands::Explain(command)) => crate::explain::execute(&cli, command),
        Some(Commands::Lint(command)) => crate::lint::execute(&cli, command),
        None => {
            if !cli.custom_command.is_empty() {
                crate::exec::execute_custom_command(&cli)
            } else {
                use clap::CommandFactory;
                crate::cli::Cli::command().print_help().ok();
                println!();
                Ok(ExitCode::SUCCESS)
            }
        }
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
