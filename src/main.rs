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
    let global_allow_env = cli.allow_env;
    let global_allow_net_out = cli.allow_net_out;

    match cli.command {
        Some(Commands::Lock) => {
            sbox::lock::lock_project()?;
            Ok(ExitCode::SUCCESS)
        },
        Some(Commands::Run {
            offline,
            allow_env,
            allow_net_out,
            command,
            args,
        }) => {
            // Verify lock before run
            sbox::lock::verify_project()?;
            execute_cmd(
                &command,
                &args,
                offline || global_offline,
                allow_env || global_allow_env,
                allow_net_out || global_allow_net_out,
            )
        },
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
            
            // For external commands (like `sbox npm start`), check lock first.
            // If they run `sbox npm install`, should we check lock?
            // Actually, we skip lock check for install-style commands in execute_cmd or here?
            // For simplicity in MVP, we check it for EVERYTHING except Lock.
            // Wait, if we check it for npm install, they can never install!
            // Let's just check it. They have to use `--force` or similar, or we just rely on `sbox lock` right after a native `npm install` for MVP.
            sbox::lock::verify_project()?;
            
            let cmd_args = &args[1..];
            execute_cmd(cmd, cmd_args, global_offline, global_allow_env, global_allow_net_out)
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

fn execute_cmd(cmd: &str, args: &[String], offline: bool, allow_env: bool, allow_net_out: bool) -> Result<ExitCode> {
    let policy = CommandPolicy::resolve(cmd, args, offline, allow_env, allow_net_out)?;

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
