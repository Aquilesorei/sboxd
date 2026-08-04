use clap::Parser;
use sbox::cli::{Cli, Commands, ShimAction};
use sbox::error::Result;
use sbox::policy::CommandPolicy;
use sbox::sandbox::NativeSandbox;
use sbox::shim;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("sbox error: {}", err);
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let global_offline = cli.offline;

    match cli.command {
        Some(Commands::Run {
            offline,
            command,
            args,
        }) => execute_cmd(&command, &args, offline || global_offline),
        Some(Commands::Shim { action }) => match action {
            ShimAction::Install => {
                shim::install()?;
                Ok(ExitCode::SUCCESS)
            }
            ShimAction::Verify => {
                shim::verify()?;
                Ok(ExitCode::SUCCESS)
            }
        },
        Some(Commands::External(args)) => {
            if args.is_empty() {
                eprintln!("Error: No command specified.");
                return Ok(ExitCode::from(1));
            }
            let cmd = &args[0];
            let cmd_args = &args[1..];
            execute_cmd(cmd, cmd_args, global_offline)
        }
        None => {
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            cmd.print_help()?;
            println!();
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn execute_cmd(cmd: &str, args: &[String], offline: bool) -> Result<ExitCode> {
    let policy = CommandPolicy::resolve(cmd, args, offline)?;

    println!(
        "[sbox] Running '{}' (network: {})",
        policy.program_name,
        if policy.network_enabled {
            "ON"
        } else {
            "OFF (unshared)"
        }
    );

    let status = NativeSandbox::execute(&policy)?;
    let code = status.code().unwrap_or(1);
    Ok(ExitCode::from(code as u8))
}
