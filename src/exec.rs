use std::process::{Command, ExitCode, Stdio};

use crate::cli::{Cli, ExecCommand, RunCommand};
use crate::config::{LoadOptions, load_config};
use crate::error::SboxError;
use crate::resolve::{ExecutionPlan, ResolutionTarget, resolve_execution_plan};

pub fn execute_run(cli: &Cli, command: &RunCommand) -> Result<ExitCode, SboxError> {
    execute(cli, ResolutionTarget::Run, &command.command)
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
    run_audit_hooks(&plan)?;

    execute_plan(&plan)
}

fn execute_plan(plan: &ExecutionPlan) -> Result<ExitCode, SboxError> {
    match plan.mode {
        crate::config::model::ExecutionMode::Host => execute_host(&plan),
        crate::config::model::ExecutionMode::Sandbox => execute_sandbox(&plan),
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

pub(crate) fn status_to_exit_code(status: std::process::ExitStatus) -> ExitCode {
    match status.code() {
        Some(code) => ExitCode::from(code as u8),
        None => ExitCode::from(1),
    }
}

fn describe_backend(backend: &crate::config::BackendKind) -> &'static str {
    match backend {
        crate::config::BackendKind::Podman => "podman",
        crate::config::BackendKind::Docker => "docker",
    }
}

pub(crate) fn execute_sandbox(plan: &ExecutionPlan) -> Result<ExitCode, SboxError> {
    match plan.backend {
        crate::config::BackendKind::Podman => crate::backend::podman::execute(plan),
        crate::config::BackendKind::Docker => Err(SboxError::SandboxExecutionNotImplemented {
            profile: plan.profile_name.clone(),
            backend: describe_backend(&plan.backend).to_string(),
        }),
    }
}

pub(crate) fn validate_execution_safety(
    plan: &ExecutionPlan,
    strict_security: bool,
) -> Result<(), SboxError> {
    if !matches!(plan.mode, crate::config::model::ExecutionMode::Sandbox) {
        return Ok(());
    }

    if trusted_image_required(plan, strict_security)
        && matches!(plan.image.trust, crate::resolve::ImageTrust::MutableReference)
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

    if plan.audit.script_hooks.applicable {
        match plan.audit.script_hooks.policy {
            crate::resolve::ScriptPolicyState::Allow => {}
            crate::resolve::ScriptPolicyState::Ignore if !plan.audit.script_hooks.blocked => {
                return Err(SboxError::UnsafeExecutionPolicy {
                    command: plan.command_string.clone(),
                    reason: "profile script policy requires install scripts to be disabled (for example with --ignore-scripts)".to_string(),
                });
            }
            crate::resolve::ScriptPolicyState::Block => {
                return Err(SboxError::UnsafeExecutionPolicy {
                    command: plan.command_string.clone(),
                    reason: "profile script policy blocks package-manager lifecycle scripts for this command".to_string(),
                });
            }
            crate::resolve::ScriptPolicyState::Ignore => {}
        }
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
                "strict security requires a lockfile for {} installs: expected {}",
                plan.audit.package_manager.as_deref().unwrap_or("package-manager"),
                plan.audit.lockfile.expected_files.join(" or ")
            ),
        });
    }

    if plan.policy.network == "off" || !plan.audit.install_style {
        return Ok(());
    }

    if plan.audit.sensitive_pass_through_vars.is_empty() {
        return Ok(());
    }

    Err(SboxError::UnsafeExecutionPolicy {
        command: plan.command_string.clone(),
        reason: format!(
            "install-style sandbox command has network enabled and sensitive pass-through vars: {}",
            plan.audit.sensitive_pass_through_vars.join(", ")
        ),
    })
}

fn run_audit_hooks(plan: &ExecutionPlan) -> Result<(), SboxError> {
    for hook in &plan.audit.audit_hooks.runnable {
        let hook_plan = ExecutionPlan {
            command: hook.command.clone(),
            command_string: hook.command.join(" "),
            ..plan.clone()
        };

        let status = execute_plan(&hook_plan)?;
        if status != ExitCode::SUCCESS {
            return Err(SboxError::AuditHookFailed {
                hook: hook.name.clone(),
                command: plan.command_string.clone(),
                status: 1,
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
    use super::{strict_security_enabled, trusted_image_required, validate_execution_safety};
    use crate::config::model::ExecutionMode;
    use crate::resolve::{
        CwdMapping, EnvVarSource, ExecutionPlan, ModeSource, ProfileSource, ResolvedEnvVar,
        ResolvedEnvironment, ResolvedImage, ResolvedImageSource, ResolvedPolicy, ResolvedUser,
        ResolvedWorkspace,
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
            audit: crate::resolve::ExecutionAudit {
                install_style: true,
                trusted_image_required: false,
                sensitive_pass_through_vars: vec!["NPM_TOKEN".into()],
                package_manager: Some("npm".into()),
                lockfile: crate::resolve::LockfileAudit {
                    applicable: true,
                    required: true,
                    present: true,
                    expected_files: vec!["package-lock.json".into()],
                },
                script_hooks: crate::resolve::ScriptHookAudit {
                    applicable: true,
                    blocked: false,
                    policy: crate::resolve::ScriptPolicyState::Allow,
                },
                audit_hooks: crate::resolve::AuditHookAudit {
                    configured: Vec::new(),
                    runnable: Vec::new(),
                },
            },
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
        let error = validate_execution_safety(&sample_plan(), true).expect_err("strict mode should reject");
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

        let error = validate_execution_safety(&plan, true).expect_err("missing lockfile should reject");
        assert!(error.to_string().contains("requires a lockfile"));
    }

    #[test]
    fn script_policy_ignore_requires_scripts_to_be_blocked() {
        let mut plan = sample_plan();
        plan.audit.sensitive_pass_through_vars.clear();
        plan.audit.script_hooks.policy = crate::resolve::ScriptPolicyState::Ignore;

        let error =
            validate_execution_safety(&plan, false).expect_err("script policy should reject");
        assert!(error
            .to_string()
            .contains("requires install scripts to be disabled"));
    }

    #[test]
    fn script_policy_block_rejects_script_capable_commands() {
        let mut plan = sample_plan();
        plan.audit.sensitive_pass_through_vars.clear();
        plan.audit.script_hooks.policy = crate::resolve::ScriptPolicyState::Block;
        plan.audit.script_hooks.blocked = true;

        let error =
            validate_execution_safety(&plan, false).expect_err("block policy should reject");
        assert!(error
            .to_string()
            .contains("blocks package-manager lifecycle scripts"));
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
            command: crate::cli::Commands::Doctor(crate::cli::DoctorCommand::default()),
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
        };

        assert!(strict_security_enabled(&cli, &config));
    }
}
