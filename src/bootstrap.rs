use std::process::ExitCode;

use crate::cli::Cli;
use crate::config::{LoadOptions, load_config};
use crate::config::model::ExecutionMode;
use crate::error::SboxError;
use crate::exec::{execute_host, execute_sandbox, run_pre_run_commands, validate_execution_safety};
use crate::resolve::{ResolutionTarget, resolve_execution_plan};

/// `sbox bootstrap` — generate the package lockfile inside the sandbox without running
/// install scripts. Requires `package_manager:` to be set in sbox.yaml.
///
/// After bootstrap succeeds, run `sbox run -- <rebuild-command>` to execute install scripts
/// with network disabled (two-phase install pattern).
pub fn execute(cli: &Cli) -> Result<ExitCode, SboxError> {
    let loaded = load_config(&LoadOptions {
        workspace: cli.workspace.clone(),
        config: cli.config.clone(),
    })?;

    let pm = loaded.config.package_manager.as_ref().ok_or_else(|| SboxError::ConfigValidation {
        message: "sbox bootstrap requires `package_manager:` in sbox.yaml\n\
                  for manually-configured profiles, generate the lockfile directly:\n\
                    sbox run -- <pm> lock  (or equivalent)"
            .to_string(),
    })?;

    let bootstrap_cmd = bootstrap_command_for(&pm.name)?;

    // Resolve against the install profile (dispatch will route the bootstrap command there).
    // Run with strict_security=false: the whole point of bootstrap is to generate the lockfile
    // that strict mode would require, so we must not refuse on its absence.
    let plan = resolve_execution_plan(cli, &loaded, ResolutionTarget::Run, &bootstrap_cmd)?;
    validate_execution_safety(&plan, false)?;
    run_pre_run_commands(&plan)?;

    eprintln!("sbox bootstrap: generating {} lockfile inside sandbox...", pm.name);

    let exit = match plan.mode {
        ExecutionMode::Host => execute_host(&plan)?,
        ExecutionMode::Sandbox => execute_sandbox(&plan)?,
    };

    if exit == ExitCode::SUCCESS {
        eprintln!("\nlockfile generated.");
        eprintln!(
            "next: sbox run -- {}",
            rebuild_hint(&pm.name)
        );
    }

    Ok(exit)
}

/// Maps package manager names to their lockfile-generation command (no scripts executed).
fn bootstrap_command_for(pm_name: &str) -> Result<Vec<String>, SboxError> {
    let cmd: &[&str] = match pm_name {
        "npm"    => &["npm",    "install", "--ignore-scripts"],
        "yarn"   => &["yarn",   "install", "--ignore-scripts"],
        "pnpm"   => &["pnpm",   "install", "--ignore-scripts"],
        "bun"    => &["bun",    "install", "--no-scripts"],
        "uv"     => &["uv",     "lock"],
        "pip"    => &["pip",    "install", "-r", "requirements.txt"],
        "poetry" => &["poetry", "lock"],
        "cargo"  => &["cargo",  "fetch"],
        "go"     => &["go",     "mod", "download"],
        _ => {
            return Err(SboxError::ConfigValidation {
                message: format!(
                    "no bootstrap command known for package manager `{pm_name}`; \
                     generate the lockfile manually and run `sbox run` directly"
                ),
            })
        }
    };
    Ok(cmd.iter().map(|s| s.to_string()).collect())
}

/// Suggests the next command to run after bootstrap (runs scripts with network off).
fn rebuild_hint(pm_name: &str) -> &'static str {
    match pm_name {
        "npm"    => "npm rebuild                  # runs install scripts, network off",
        "yarn"   => "yarn install                 # runs install scripts, network off",
        "pnpm"   => "pnpm rebuild                 # runs install scripts, network off",
        "bun"    => "bun install                  # runs install scripts, network off",
        "uv"     => "uv sync                      # install from lockfile, network off",
        "pip"    => "pip install -r requirements.txt  # install from requirements",
        "poetry" => "poetry install               # install from lockfile, network off",
        "cargo"  => "cargo build                  # compile from fetched sources",
        "go"     => "go build ./...               # compile from downloaded modules",
        _        => "<install-command>",
    }
}
