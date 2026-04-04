use std::process::ExitCode;

use crate::cli::{Cli, ShellCommand};
use crate::config::{LoadOptions, load_config};
use crate::error::SboxError;
use crate::resolve::{ResolutionTarget, resolve_execution_plan};

pub fn execute(cli: &Cli, command: &ShellCommand) -> Result<ExitCode, SboxError> {
    let loaded = load_config(&LoadOptions {
        workspace: cli.workspace.clone(),
        config: cli.config.clone(),
    })?;

    let initial_shell = resolve_initial_shell(command)?;
    let initial_plan = resolve_execution_plan(
        cli,
        &loaded,
        ResolutionTarget::Shell,
        std::slice::from_ref(&initial_shell),
    )?;
    let shell = resolve_shell(cli, command, &loaded.config, &initial_plan.profile_name)?;
    let plan = if shell == initial_shell {
        initial_plan
    } else {
        resolve_execution_plan(cli, &loaded, ResolutionTarget::Shell, &[shell])?
    };
    crate::exec::validate_execution_safety(
        &plan,
        crate::exec::strict_security_enabled(cli, &loaded.config),
    )?;

    match plan.mode {
        crate::config::model::ExecutionMode::Host => crate::exec::execute_host(&plan),
        crate::config::model::ExecutionMode::Sandbox => match plan.backend {
            crate::config::BackendKind::Podman => {
                crate::backend::podman::execute_interactive(&plan)
            }
            crate::config::BackendKind::Docker => {
                crate::backend::docker::execute_interactive(&plan)
            }
        },
    }
}

fn resolve_shell(
    cli: &Cli,
    command: &ShellCommand,
    config: &crate::config::model::Config,
    active_profile: &str,
) -> Result<String, SboxError> {
    if let Some(shell) = &command.shell {
        return Ok(shell.clone());
    }

    if let Some(profile) = config.profiles.get(active_profile)
        && let Some(shell) = &profile.shell
    {
        return Ok(shell.clone());
    }

    if let Some(profile_name) = &cli.profile
        && let Some(profile) = config.profiles.get(profile_name)
        && let Some(shell) = &profile.shell
    {
        return Ok(shell.clone());
    }

    if let Some(shell) = std::env::var_os("SHELL") {
        let shell = shell.to_string_lossy().trim().to_string();
        if !shell.is_empty() {
            return Ok(shell);
        }
    }

    Ok("/bin/sh".to_string())
}

fn resolve_initial_shell(command: &ShellCommand) -> Result<String, SboxError> {
    if let Some(shell) = &command.shell {
        return Ok(shell.clone());
    }

    if let Some(shell) = std::env::var_os("SHELL") {
        let shell = shell.to_string_lossy().trim().to_string();
        if !shell.is_empty() {
            return Ok(shell);
        }
    }

    let fallback = "/bin/sh".to_string();
    if fallback.is_empty() {
        Err(SboxError::NoShellResolved)
    } else {
        Ok(fallback)
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::resolve_shell;
    use crate::cli::{Cli, Commands, ShellCommand};
    use crate::config::model::{Config, ExecutionMode, ProfileConfig};

    fn base_cli() -> Cli {
        Cli {
            config: None,
            workspace: None,
            backend: None,
            image: None,
            profile: None,
            mode: None,
            strict_security: false,
            verbose: 0,
            quiet: false,
            command: Commands::Shell(ShellCommand::default()),
        }
    }

    fn base_config() -> Config {
        let mut profiles = IndexMap::new();
        profiles.insert(
            "default".to_string(),
            ProfileConfig {
                mode: ExecutionMode::Sandbox,
                image: None,
                network: Some("off".to_string()),
                writable: Some(true),
                require_pinned_image: None,
                require_lockfile: None,
            role: None,
            lockfile_files: Vec::new(),
            pre_run: Vec::new(),
            network_allow: Vec::new(),
                ports: Vec::new(),
                capabilities: None,
                no_new_privileges: Some(true),
                read_only_rootfs: None,
                reuse_container: None,
                shell: Some("/bin/bash".to_string()),
                writable_paths: None,
            },
        );

        Config {
            version: 1,
            runtime: None,
            workspace: None,
            identity: None,
            image: None,
            environment: None,
            mounts: Vec::new(),
            caches: Vec::new(),
            secrets: Vec::new(),
            profiles,
            dispatch: IndexMap::new(),

            package_manager: None,
        }
    }

    #[test]
    fn shell_flag_overrides_profile_shell() {
        let cli = base_cli();
        let config = base_config();
        let command = ShellCommand {
            shell: Some("/bin/zsh".to_string()),
        };

        let shell =
            resolve_shell(&cli, &command, &config, "default").expect("shell should resolve");
        assert_eq!(shell, "/bin/zsh");
    }

    #[test]
    fn profile_shell_is_used_when_flag_is_absent() {
        let cli = base_cli();
        let config = base_config();

        let shell = resolve_shell(&cli, &ShellCommand::default(), &config, "default")
            .expect("shell should resolve");
        assert_eq!(shell, "/bin/bash");
    }
}
