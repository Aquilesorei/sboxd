use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

use crate::cli::{Cli, ExecCommand, RunCommand};
use crate::config::{BackendKind, LoadOptions, load_config};
use crate::error::SboxError;
use crate::resolve::{
    EnvVarSource, ExecutionPlan, ResolutionTarget, ResolvedEnvVar, resolve_execution_plan,
};
use crate::shadow::{detect_infrastructure, resolve_shadow_plan};

pub fn execute_run(cli: &Cli, command: &RunCommand) -> Result<ExitCode, SboxError> {
    let load_options = LoadOptions {
        workspace: cli.workspace.clone(),
        config: cli.config.clone(),
    };

    let loaded_res = load_config(&load_options);

    match loaded_res {
        Ok(loaded) => {
            let mut plan = resolve_execution_plan(cli, &loaded, ResolutionTarget::Run, &command.command)?;
            apply_env_overrides(&mut plan, &command.env)?;
            run_plan(cli, &plan, command.dry_run, Some(&loaded))
        }
        Err(SboxError::ConfigNotFound(_)) if cli.config.is_none() => {
            // Shadow Mode
            let invocation_dir = std::env::current_dir().map_err(|e| SboxError::CurrentDirectory { source: e })?;
            let workspace_root = crate::config::load::absolutize_path(cli.workspace.as_deref(), &invocation_dir)
                .unwrap_or(invocation_dir.clone());
            
            let backend = match cli.backend {
                Some(crate::cli::CliBackendKind::Podman) => BackendKind::Podman,
                Some(crate::cli::CliBackendKind::Docker) => BackendKind::Docker,
                None => {
                    // Check environment variable
                    match std::env::var("SBOX_BACKEND").as_deref() {
                        Ok("podman") => BackendKind::Podman,
                        Ok("docker") => BackendKind::Docker,
                        _ => {
                            // Default detection
                            if std::process::Command::new("podman").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok() {
                                BackendKind::Podman
                            } else {
                                BackendKind::Docker
                            }
                        }
                    }
                }
            };

            let infra = detect_infrastructure(&workspace_root, backend.clone());
            let mut plan = resolve_shadow_plan(infra, command.command.clone(), &workspace_root, &invocation_dir, backend)?;
            apply_env_overrides(&mut plan, &command.env)?;
            run_plan(cli, &plan, command.dry_run, None)
        }
        Err(e) => Err(e),
    }
}

fn run_plan(cli: &Cli, plan: &ExecutionPlan, dry_run: bool, loaded: Option<&crate::config::load::LoadedConfig>) -> Result<ExitCode, SboxError> {
    if dry_run {
        let config_path = loaded.map(|l| l.config_path.as_path()).unwrap_or_else(|| std::path::Path::new("shadow"));
        let strict = loaded.map(|l| strict_security_enabled(cli, &l.config)).unwrap_or(cli.strict_security);
        print!(
            "{}",
            crate::plan::render_plan(
                config_path,
                plan,
                strict,
                true,
                false
            )
        );
        return Ok(ExitCode::SUCCESS);
    }

    let strict = loaded.map(|l| strict_security_enabled(cli, &l.config)).unwrap_or(cli.strict_security);
    validate_execution_safety(plan, strict)?;
    run_pre_run_commands(plan)?;
    let compose_cleanup = run_compose_up(plan)?;
    let result = execute_plan(plan);
    if let Some(cleanup) = compose_cleanup {
        let _ = cleanup.run();
    }
    result
}

pub fn execute_exec(cli: &Cli, command: &ExecCommand) -> Result<ExitCode, SboxError> {
    execute(
        cli,
        ResolutionTarget::Exec {
            profile: &command.profile,
        },
        &command.command,
    )
}

fn execute(
    cli: &Cli,
    target: ResolutionTarget<'_>,
    command: &[String],
) -> Result<ExitCode, SboxError> {
    let loaded = load_config(&LoadOptions {
        workspace: cli.workspace.clone(),
        config: cli.config.clone(),
    })?;
    let plan = resolve_execution_plan(cli, &loaded, target, command)?;
    validate_execution_safety(&plan, strict_security_enabled(cli, &loaded.config))?;
    run_pre_run_commands(&plan)?;
    let compose_cleanup = run_compose_up(&plan)?;
    let result = execute_plan(&plan);
    if let Some(cleanup) = compose_cleanup {
        let _ = cleanup.run();
    }
    result
}

pub fn execute_custom_command(cli: &Cli) -> Result<ExitCode, SboxError> {
    let load_options = LoadOptions {
        workspace: cli.workspace.clone(),
        config: cli.config.clone(),
    };

    let loaded_res = load_config(&load_options);

    match loaded_res {
        Ok(loaded) => {
            let name = &cli.custom_command[0];
            let config_command =
                loaded
                    .config
                    .commands
                    .get(name)
                    .ok_or_else(|| SboxError::UnknownCommand {
                        name: name.clone(),
                    })?;

            let mut full_command = config_command.run.clone();
            full_command.extend(cli.custom_command.iter().skip(1).cloned());

            let target = match &config_command.profile {
                Some(profile) => ResolutionTarget::Exec { profile },
                None => ResolutionTarget::Run,
            };

            let plan = resolve_execution_plan(cli, &loaded, target, &full_command)?;

            run_plan(cli, &plan, false, Some(&loaded))
        }
        Err(SboxError::ConfigNotFound(_)) if cli.config.is_none() => {
            // Shadow Mode: if we got here, it's because it was an unknown subcommand that might be a command to run directly in shadow mode
            let invocation_dir = std::env::current_dir().map_err(|e| SboxError::CurrentDirectory { source: e })?;
            let workspace_root = crate::config::load::absolutize_path(cli.workspace.as_deref(), &invocation_dir)
                .unwrap_or(invocation_dir.clone());
            
            let backend = match cli.backend {
                Some(crate::cli::CliBackendKind::Podman) => BackendKind::Podman,
                Some(crate::cli::CliBackendKind::Docker) => BackendKind::Docker,
                None => {
                    match std::env::var("SBOX_BACKEND").as_deref() {
                        Ok("podman") => BackendKind::Podman,
                        Ok("docker") => BackendKind::Docker,
                        _ => {
                            if std::process::Command::new("podman").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok() {
                                BackendKind::Podman
                            } else {
                                BackendKind::Docker
                            }
                        }
                    }
                }
            };

            let infra = detect_infrastructure(&workspace_root, backend.clone());
            let plan = resolve_shadow_plan(infra, cli.custom_command.clone(), &workspace_root, &invocation_dir, backend)?;
            run_plan(cli, &plan, false, None)
        }
        Err(e) => Err(e),
    }
}

fn apply_env_overrides(plan: &mut ExecutionPlan, overrides: &[String]) -> Result<(), SboxError> {
    for entry in overrides {
        let (name, value) = entry
            .split_once('=')
            .ok_or_else(|| SboxError::ConfigValidation {
                message: format!("-e `{entry}` must be in NAME=VALUE format"),
            })?;
        if !is_valid_env_name(name) {
            return Err(SboxError::ConfigValidation {
                message: format!(
                    "-e `{entry}`: `{name}` is not a valid environment variable name \
                     (must be non-empty, start with a letter or underscore, and contain \
                     only letters, digits, or underscores)"
                ),
            });
        }
        // Remove any existing entry with the same name so the override wins
        plan.environment.variables.retain(|v| v.name != name);
        plan.environment.variables.push(ResolvedEnvVar {
            name: name.to_string(),
            value: value.to_string(),
            source: EnvVarSource::Set,
        });
    }
    Ok(())
}

fn is_valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        None => false,
        Some(first) => {
            (first.is_ascii_alphabetic() || first == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
    }
}

fn execute_plan(plan: &ExecutionPlan) -> Result<ExitCode, SboxError> {
    match plan.mode {
        crate::config::model::ExecutionMode::Host => execute_host(plan),
        crate::config::model::ExecutionMode::Sandbox => execute_sandbox(plan),
    }
}

pub(crate) fn execute_host(plan: &ExecutionPlan) -> Result<ExitCode, SboxError> {
    let (program, args) = plan
        .command
        .split_first()
        .expect("command vectors are validated by clap");

    let mut child = Command::new(program);
    child.args(args);
    child.current_dir(&plan.workspace.effective_host_dir);
    child.stdin(Stdio::inherit());
    child.stdout(Stdio::inherit());
    child.stderr(Stdio::inherit());

    for denied in &plan.environment.denied {
        child.env_remove(denied);
    }

    for variable in &plan.environment.variables {
        child.env(&variable.name, &variable.value);
    }

    let status = child.status().map_err(|source| SboxError::CommandSpawn {
        program: program.clone(),
        source,
    })?;

    Ok(status_to_exit_code(status))
}

pub fn status_to_exit_code(status: std::process::ExitStatus) -> ExitCode {
    match status.code() {
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => ExitCode::from(1),
    }
}

pub(crate) fn execute_sandbox(plan: &ExecutionPlan) -> Result<ExitCode, SboxError> {
    match plan.backend {
        crate::config::BackendKind::Podman => crate::backend::podman::execute(plan),
        crate::config::BackendKind::Docker => crate::backend::docker::execute(plan),
    }
}

pub(crate) fn validate_execution_safety(
    plan: &ExecutionPlan,
    strict_security: bool,
) -> Result<(), SboxError> {
    if !matches!(plan.mode, crate::config::model::ExecutionMode::Sandbox) {
        return Ok(());
    }

    if plan.policy.network_policy == crate::config::model::NetworkPolicy::Firewall {
        if plan.policy.network == "off" {
            return Ok(());
        }

        if !cfg!(target_os = "linux") {
            return Err(SboxError::FirewallPolicyUnavailable {
                reason: "`network_policy: firewall` is only supported on Linux".to_string(),
            });
        }

        if plan.rootless {
            return Err(SboxError::FirewallPolicyUnavailable {
                reason: "`network_policy: firewall` requires `runtime.rootless: false`".to_string(),
            });
        }

        if unsafe { libc::geteuid() } != 0 {
            return Err(SboxError::FirewallPolicyUnavailable {
                reason: "`network_policy: firewall` requires running sbox as root (e.g. via sudo)".to_string(),
            });
        }
    }

    if trusted_image_required(plan, strict_security)
        && matches!(
            plan.image.trust,
            crate::resolve::ImageTrust::MutableReference
        )
    {
        return Err(SboxError::UnsafeExecutionPolicy {
            command: plan.command_string.clone(),
            reason: "strict security requires a pinned image digest or local build for sandbox execution".to_string(),
        });
    }

    if strict_security && !plan.audit.sensitive_pass_through_vars.is_empty() {
        return Err(SboxError::UnsafeExecutionPolicy {
            command: plan.command_string.clone(),
            reason: format!(
                "strict security forbids sensitive host pass-through vars in sandbox mode: {}",
                plan.audit.sensitive_pass_through_vars.join(", ")
            ),
        });
    }

    if strict_security
        && plan.audit.install_style
        && plan.audit.lockfile.applicable
        && plan.audit.lockfile.required
        && !plan.audit.lockfile.present
    {
        return Err(SboxError::UnsafeExecutionPolicy {
            command: plan.command_string.clone(),
            reason: format!(
                "strict security requires a lockfile for install-style commands: expected {}",
                plan.audit.lockfile.expected_files.join(" or ")
            ),
        });
    }

    if plan.policy.network == "off" || !plan.audit.install_style {
        validate_firewall_policy(plan)?;
        return Ok(());
    }

    if plan.audit.sensitive_pass_through_vars.is_empty() {
        validate_firewall_policy(plan)?;
        return Ok(());
    }

    validate_firewall_policy(plan)?;
    Err(SboxError::UnsafeExecutionPolicy {
        command: plan.command_string.clone(),
        reason: format!(
            "install-style sandbox command has network enabled and sensitive pass-through vars: {}",
            plan.audit.sensitive_pass_through_vars.join(", ")
        ),
    })
}

fn validate_firewall_policy(plan: &ExecutionPlan) -> Result<(), SboxError> {
    if plan.policy.network_policy != crate::config::model::NetworkPolicy::Firewall {
        return Ok(());
    }

    #[cfg(not(target_os = "linux"))]
    {
        return Err(SboxError::FirewallPolicyUnavailable {
            reason: "firewall network policy is only supported on Linux".to_string(),
        });
    }

    #[cfg(target_os = "linux")]
    {
        if plan.rootless {
            return Err(SboxError::FirewallPolicyUnavailable {
                reason: "firewall network policy requires rootful execution (rootless: false)".to_string(),
            });
        }

        if unsafe { libc::getuid() } != 0 {
            return Err(SboxError::FirewallPolicyUnavailable {
                reason: "firewall network policy requires root privileges (run with sudo)".to_string(),
            });
        }
    }

    Ok(())
}

pub(crate) fn run_compose_up(plan: &ExecutionPlan) -> Result<Option<ComposeCleanup>, SboxError> {
    let Some(compose) = &plan.compose else {
        return Ok(None);
    };

    let (backend, mut args) = compose_command(plan.backend.clone(), &compose.file, "up");
    args.extend([
        "-d".to_string(),
    ]);
    if !compose.services.is_empty() {
        args.extend(compose.services.iter().cloned());
    }

    println!("Starting compose services...");
    let status = Command::new(&backend)
        .args(&args)
        .current_dir(&plan.workspace.root)
        .status()
        .map_err(|source| SboxError::CommandSpawn {
            program: backend.clone(),
            source,
        })?;

    if !status.success() {
        return Err(SboxError::BackendCommandFailed {
            backend: backend.clone(),
            command: format!("{} {}", backend, args.join(" ")),
            status: status.code().unwrap_or(1),
        });
    }

    Ok(Some(ComposeCleanup {
        backend,
        file: compose.file.clone(),
        services: compose.services.clone(),
    }))
}

fn compose_command(
    backend: crate::config::BackendKind,
    file: &std::path::Path,
    verb: &str,
) -> (String, Vec<String>) {
    match backend {
        crate::config::BackendKind::Podman => (
            "podman-compose".to_string(),
            vec![
                "-f".to_string(),
                file.display().to_string(),
                verb.to_string(),
            ],
        ),
        crate::config::BackendKind::Docker => (
            "docker".to_string(),
            vec![
                "compose".to_string(),
                "-f".to_string(),
                file.display().to_string(),
                verb.to_string(),
            ],
        ),
    }
}

pub struct ComposeCleanup {
    backend: String,
    file: PathBuf,
    services: Vec<String>,
}

impl ComposeCleanup {
    pub fn run(self) -> Result<(), SboxError> {
        let backend_kind = if self.backend == "docker" {
            crate::config::BackendKind::Docker
        } else {
            crate::config::BackendKind::Podman
        };
        let (backend, mut args) = compose_command(backend_kind, &self.file, "stop");
        if !self.services.is_empty() {
            args.extend(self.services);
        }

        let _ = Command::new(&backend).args(&args).status();
        Ok(())
    }
}

pub(crate) fn run_pre_run_commands(plan: &ExecutionPlan) -> Result<(), SboxError> {
    for argv in &plan.audit.pre_run {
        let (program, args) = argv
            .split_first()
            .expect("pre_run commands are non-empty after parse");

        let status = Command::new(program)
            .args(args)
            .current_dir(&plan.workspace.effective_host_dir)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|source| SboxError::CommandSpawn {
                program: program.clone(),
                source,
            })?;

        if !status.success() {
            return Err(SboxError::PreRunFailed {
                pre_run: argv.join(" "),
                command: plan.command_string.clone(),
                status: status.code().unwrap_or(1) as u8,
            });
        }
    }

    Ok(())
}

pub(crate) fn trusted_image_required(plan: &ExecutionPlan, strict_security: bool) -> bool {
    matches!(plan.mode, crate::config::model::ExecutionMode::Sandbox)
        && (strict_security || plan.audit.trusted_image_required)
}

pub(crate) fn strict_security_enabled(cli: &Cli, config: &crate::config::model::Config) -> bool {
    cli.strict_security
        || config
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.strict_security)
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{
        apply_env_overrides, compose_command, is_valid_env_name, strict_security_enabled,
        trusted_image_required, validate_execution_safety,
    };
    use crate::config::model::ExecutionMode;
    use crate::resolve::{
        CwdMapping, EnvVarSource, ExecutionAudit, ExecutionPlan, LockfileAudit, ModeSource,
        ProfileSource, ResolvedEnvVar, ResolvedEnvironment, ResolvedImage, ResolvedImageSource,
        ResolvedPolicy, ResolvedUser, ResolvedWorkspace,
    };
    use std::path::PathBuf;

    fn sample_plan() -> ExecutionPlan {
        ExecutionPlan {
            command: vec!["npm".into(), "install".into()],
            command_string: "npm install".into(),
            backend: crate::config::BackendKind::Podman,
            image: ResolvedImage {
                description: "ref:node:22-bookworm-slim".into(),
                source: ResolvedImageSource::Reference("node:22-bookworm-slim".into()),
                trust: crate::resolve::ImageTrust::MutableReference,
                verify_signature: false,
            },
            profile_name: "install".into(),
            profile_source: ProfileSource::DefaultProfile,
            mode: ExecutionMode::Sandbox,
            mode_source: ModeSource::Profile,
            workspace: ResolvedWorkspace {
                root: PathBuf::from("/tmp/project"),
                invocation_dir: PathBuf::from("/tmp/project"),
                effective_host_dir: PathBuf::from("/tmp/project"),
                mount: "/workspace".into(),
                sandbox_cwd: "/workspace".into(),
                cwd_mapping: CwdMapping::InvocationMapped,
            },
            policy: ResolvedPolicy {
                network: "on".into(),
                writable: true,
                ports: Vec::new(),
                no_new_privileges: true,
                read_only_rootfs: false,
                reuse_container: false,
                reusable_session_name: None,
                cap_drop: Vec::new(),
                cap_add: Vec::new(),
                pull_policy: None,
                network_allow: Vec::new(),
                network_allow_patterns: Vec::new(),
                network_policy: crate::config::model::NetworkPolicy::Dns,
            },
            environment: ResolvedEnvironment {
                variables: vec![ResolvedEnvVar {
                    name: "NPM_TOKEN".into(),
                    value: "secret".into(),
                    source: EnvVarSource::PassThrough,
                }],
                denied: Vec::new(),
            },
            mounts: Vec::new(),
            caches: Vec::new(),
            secrets: Vec::new(),
            user: ResolvedUser::KeepId,
            rootless: true,
            audit: ExecutionAudit {
                install_style: true,
                trusted_image_required: false,
                sensitive_pass_through_vars: vec!["NPM_TOKEN".into()],
                lockfile: LockfileAudit {
                    applicable: true,
                    required: true,
                    present: true,
                    expected_files: vec!["package-lock.json".into()],
                },
                pre_run: Vec::new(),
            },
            compose: None,
        }
    }

    #[test]
    fn rejects_networked_install_with_sensitive_pass_through_envs() {
        let error =
            validate_execution_safety(&sample_plan(), false).expect_err("policy should reject");
        assert!(error.to_string().contains("unsafe sandbox execution"));
    }

    #[test]
    fn allows_networked_install_without_sensitive_pass_through_envs() {
        let mut plan = sample_plan();
        plan.audit.sensitive_pass_through_vars.clear();
        validate_execution_safety(&plan, false).expect("policy should allow");
    }

    #[test]
    fn strict_security_rejects_sensitive_pass_through_even_without_install_pattern() {
        let mut plan = sample_plan();
        plan.command = vec!["node".into(), "--version".into()];
        plan.command_string = "node --version".into();
        plan.audit.install_style = false;

        let error = validate_execution_safety(&plan, true).expect_err("strict mode should reject");
        assert!(error.to_string().contains("requires a pinned image digest"));
    }

    #[test]
    fn strict_security_requires_trusted_image() {
        let error =
            validate_execution_safety(&sample_plan(), true).expect_err("strict mode should reject");
        assert!(error.to_string().contains("pinned image digest"));
    }

    #[test]
    fn strict_security_allows_pinned_image() {
        let mut plan = sample_plan();
        plan.image.source =
            ResolvedImageSource::Reference("node:22-bookworm-slim@sha256:deadbeef".into());
        plan.image.trust = crate::resolve::ImageTrust::PinnedDigest;
        plan.audit.sensitive_pass_through_vars.clear();

        validate_execution_safety(&plan, true).expect("strict mode should allow pinned images");
    }

    #[test]
    fn strict_security_marks_trusted_image_requirement() {
        assert!(trusted_image_required(&sample_plan(), true));
        assert!(!trusted_image_required(&sample_plan(), false));
    }

    #[test]
    fn profile_policy_requires_trusted_image_without_strict_mode() {
        let mut plan = sample_plan();
        plan.audit.trusted_image_required = true;

        let error =
            validate_execution_safety(&plan, false).expect_err("profile policy should reject");
        assert!(error.to_string().contains("pinned image digest"));
    }

    #[test]
    fn strict_security_requires_lockfile_for_install_flows() {
        let mut plan = sample_plan();
        plan.image.source =
            ResolvedImageSource::Reference("node:22-bookworm-slim@sha256:deadbeef".into());
        plan.image.trust = crate::resolve::ImageTrust::PinnedDigest;
        plan.audit.sensitive_pass_through_vars.clear();
        plan.audit.lockfile.present = false;

        let error =
            validate_execution_safety(&plan, true).expect_err("missing lockfile should reject");
        assert!(
            error
                .to_string()
                .contains("requires a lockfile for install-style")
        );
    }

    #[test]
    fn docker_compose_uses_docker_cli_subcommand() {
        let (program, args) = compose_command(
            crate::config::BackendKind::Docker,
            std::path::Path::new("docker-compose.yml"),
            "up",
        );
        assert_eq!(program, "docker");
        assert_eq!(args, vec!["compose", "-f", "docker-compose.yml", "up"]);
    }

    #[test]
    fn env_override_replaces_existing_variable() {
        let mut plan = sample_plan();
        plan.environment.variables.push(ResolvedEnvVar {
            name: "MY_VAR".into(),
            value: "old".into(),
            source: EnvVarSource::Set,
        });
        apply_env_overrides(&mut plan, &["MY_VAR=new".to_string()])
            .expect("override should succeed");
        let vars: Vec<_> = plan
            .environment
            .variables
            .iter()
            .filter(|v| v.name == "MY_VAR")
            .collect();
        assert_eq!(vars.len(), 1, "duplicate must be removed");
        assert_eq!(vars[0].value, "new");
    }

    #[test]
    fn env_override_accepts_multiple_entries() {
        let mut plan = sample_plan();
        apply_env_overrides(&mut plan, &["FOO=bar".to_string(), "BAZ=qux".to_string()])
            .expect("multiple overrides should succeed");
        let names: Vec<_> = plan
            .environment
            .variables
            .iter()
            .map(|v| v.name.as_str())
            .collect();
        assert!(names.contains(&"FOO"));
        assert!(names.contains(&"BAZ"));
    }

    #[test]
    fn env_override_rejects_missing_equals() {
        let mut plan = sample_plan();
        let result = apply_env_overrides(&mut plan, &["NOEQUALS".to_string()]);
        assert!(result.is_err(), "missing = must be rejected");
    }

    #[test]
    fn env_override_rejects_invalid_name() {
        let mut plan = sample_plan();
        // Names starting with a digit are invalid
        assert!(apply_env_overrides(&mut plan, &["1FOO=bar".to_string()]).is_err());
        // Names with hyphens are invalid
        assert!(apply_env_overrides(&mut plan, &["MY-VAR=val".to_string()]).is_err());
        // Empty name is invalid
        assert!(apply_env_overrides(&mut plan, &["=value".to_string()]).is_err());
    }

    #[test]
    fn is_valid_env_name_accepts_valid_names() {
        assert!(is_valid_env_name("FOO"));
        assert!(is_valid_env_name("_PRIVATE"));
        assert!(is_valid_env_name("MY_VAR_123"));
        assert!(is_valid_env_name("a"));
    }

    #[test]
    fn is_valid_env_name_rejects_invalid_names() {
        assert!(!is_valid_env_name(""));
        assert!(!is_valid_env_name("1FOO"));
        assert!(!is_valid_env_name("MY-VAR"));
        assert!(!is_valid_env_name("MY VAR"));
        assert!(!is_valid_env_name("MY.VAR"));
    }

    #[test]
    fn cli_flag_enables_strict_security() {
        let cli = crate::cli::Cli {
            config: None,
            workspace: None,
            backend: None,
            image: None,
            profile: None,
            mode: None,
            verbose: 0,
            quiet: false,
            strict_security: true,
            output_format: crate::cli::OutputFormat::Text,
            command: Some(crate::cli::Commands::Doctor(
                crate::cli::DoctorCommand::default(),
            )),
            custom_command: Vec::new(),
        };
        let config = crate::config::model::Config {
            version: 1,
            runtime: None,
            workspace: None,
            identity: None,
            image: None,
            environment: None,
            mounts: Vec::new(),
            caches: Vec::new(),
            secrets: Vec::new(),
            profiles: Default::default(),
            dispatch: Default::default(),

            package_manager: None,
            commands: Default::default(),
        };

        assert!(strict_security_enabled(&cli, &config));
    }
}
