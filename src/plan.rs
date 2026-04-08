use std::fmt::Write as _;
use std::process::ExitCode;

use crate::cli::{Cli, PlanCommand};
use crate::config::{LoadOptions, load_config};
use crate::error::SboxError;
use crate::resolve::{
    CwdMapping, EnvVarSource, ExecutionPlan, ModeSource, ProfileSource, ResolutionTarget,
    ResolvedImageSource, resolve_execution_plan,
};

pub fn execute(cli: &Cli, command: &PlanCommand) -> Result<ExitCode, SboxError> {
    let loaded = load_config(&LoadOptions {
        workspace: cli.workspace.clone(),
        config: cli.config.clone(),
    })?;

    let (target, effective_command): (ResolutionTarget<'_>, Vec<String>) =
        if command.command.is_empty() {
            let profile =
                cli.profile
                    .as_deref()
                    .ok_or_else(|| SboxError::ProfileResolutionFailed {
                        command: "<none>".to_string(),
                    })?;
            (
                ResolutionTarget::Exec { profile },
                vec!["<profile-inspection>".to_string()],
            )
        } else {
            (ResolutionTarget::Plan, command.command.clone())
        };

    let plan = resolve_execution_plan(cli, &loaded, target, &effective_command)?;
    let strict_security = crate::exec::strict_security_enabled(cli, &loaded.config);

    match cli.output_format {
        crate::cli::OutputFormat::Json => {
            let obj = plan_to_json(&loaded.config_path, &plan, strict_security);
            println!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
        }
        crate::cli::OutputFormat::Text => {
            print!(
                "{}",
                render_plan(
                    &loaded.config_path,
                    &plan,
                    strict_security,
                    command.show_command,
                    command.command.is_empty(),
                )
            );
        }
    }

    // Append an inline security scan when the user passes --audit.
    // Gated behind a flag so `sbox plan` stays fast and deterministic by default.
    // Non-blocking: a missing audit tool or tool failure never causes `sbox plan` to exit non-zero.
    if command.audit && !command.command.is_empty() {
        let pm_name = loaded
            .config
            .package_manager
            .as_ref()
            .map(|pm| pm.name.as_str())
            .unwrap_or_else(|| crate::audit::detect_pm_from_workspace(&loaded.workspace_root));

        let result = crate::audit::run_inline(pm_name, &loaded.workspace_root);
        print!("{}", render_inline_audit(&result));
    }

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn render_plan(
    config_path: &std::path::Path,
    plan: &ExecutionPlan,
    strict_security: bool,
    show_command: bool,
    profile_inspection: bool,
) -> String {
    let mut output = String::new();
    writeln!(output, "sbox plan").ok();
    writeln!(output, "phase: 2").ok();
    writeln!(output, "config: {}", config_path.display()).ok();
    writeln!(output).ok();

    if profile_inspection {
        writeln!(output, "command: <profile inspection — no command given>").ok();
    } else {
        writeln!(output, "command: {}", plan.command_string).ok();
        writeln!(output, "argv:").ok();
        for arg in &plan.command {
            writeln!(output, "  - {arg}").ok();
        }
    }
    writeln!(output).ok();

    if profile_inspection {
        writeln!(output, "audit: <not applicable for profile inspection>").ok();
        writeln!(output).ok();
    } else {
        writeln!(output, "audit:").ok();
        writeln!(output, "  install_style: {}", plan.audit.install_style).ok();
        writeln!(output, "  strict_security: {}", strict_security).ok();
        writeln!(
            output,
            "  trusted_image_required: {}",
            crate::exec::trusted_image_required(plan, strict_security)
        )
        .ok();
        writeln!(
            output,
            "  sensitive_pass_through: {}",
            if plan.audit.sensitive_pass_through_vars.is_empty() {
                "<none>".to_string()
            } else {
                plan.audit.sensitive_pass_through_vars.join(", ")
            }
        )
        .ok();
        writeln!(
            output,
            "  lockfile: {}",
            describe_lockfile_audit(&plan.audit.lockfile)
        )
        .ok();
        writeln!(
            output,
            "  pre_run: {}",
            describe_pre_run(&plan.audit.pre_run)
        )
        .ok();
        writeln!(output).ok();
    } // end audit block

    writeln!(output, "resolution:").ok();
    writeln!(output, "  profile: {}", plan.profile_name).ok();
    writeln!(
        output,
        "  profile source: {}",
        describe_profile_source(&plan.profile_source)
    )
    .ok();
    writeln!(output, "  mode: {}", describe_execution_mode(&plan.mode)).ok();
    writeln!(
        output,
        "  mode source: {}",
        describe_mode_source(&plan.mode_source)
    )
    .ok();
    writeln!(output).ok();

    writeln!(output, "runtime:").ok();
    writeln!(output, "  backend: {}", describe_backend(&plan.backend)).ok();
    writeln!(output, "  image: {}", plan.image.description).ok();
    writeln!(
        output,
        "  image_trust: {}",
        describe_image_trust(plan.image.trust)
    )
    .ok();
    writeln!(
        output,
        "  verify_signature: {}",
        if plan.image.verify_signature {
            "requested"
        } else {
            "not requested"
        }
    )
    .ok();
    writeln!(output, "  user mapping: {}", describe_user(&plan.user)).ok();
    writeln!(output).ok();

    writeln!(output, "workspace:").ok();
    writeln!(output, "  root: {}", plan.workspace.root.display()).ok();
    writeln!(
        output,
        "  invocation cwd: {}",
        plan.workspace.invocation_dir.display()
    )
    .ok();
    writeln!(
        output,
        "  effective host dir: {}",
        plan.workspace.effective_host_dir.display()
    )
    .ok();
    writeln!(output, "  mount: {}", plan.workspace.mount).ok();
    writeln!(output, "  sandbox cwd: {}", plan.workspace.sandbox_cwd).ok();
    writeln!(
        output,
        "  cwd mapping: {}",
        describe_cwd_mapping(&plan.workspace.cwd_mapping)
    )
    .ok();
    writeln!(output).ok();

    writeln!(output, "policy:").ok();
    writeln!(output, "  network: {}", plan.policy.network).ok();
    writeln!(
        output,
        "  network_policy: {}",
        match plan.policy.network_policy {
            crate::config::model::NetworkPolicy::Dns => "dns",
            crate::config::model::NetworkPolicy::Firewall => "firewall",
        }
    )
    .ok();
    writeln!(
        output,
        "  network_allow: {}",
        describe_network_allow(
            &plan.policy.network_allow,
            &plan.policy.network_allow_patterns
        )
    )
    .ok();
    if plan.policy.network != "off"
        && (!plan.policy.network_allow.is_empty() || !plan.policy.network_allow_patterns.is_empty())
    {
        writeln!(
            output,
            "  note: `network_allow` is hostname/DNS-based and does not block direct IP connections"
        )
        .ok();
    }
    writeln!(output, "  writable: {}", plan.policy.writable).ok();
    writeln!(
        output,
        "  no_new_privileges: {}",
        plan.policy.no_new_privileges
    )
    .ok();
    writeln!(
        output,
        "  read_only_rootfs: {}",
        plan.policy.read_only_rootfs
    )
    .ok();
    writeln!(output, "  reuse_container: {}", plan.policy.reuse_container).ok();
    writeln!(
        output,
        "  reusable_session: {}",
        plan.policy
            .reusable_session_name
            .as_deref()
            .unwrap_or("<none>")
    )
    .ok();
    writeln!(
        output,
        "  cap_drop: {}",
        if plan.policy.cap_drop.is_empty() {
            "<none>".to_string()
        } else {
            plan.policy.cap_drop.join(", ")
        }
    )
    .ok();
    writeln!(
        output,
        "  cap_add: {}",
        if plan.policy.cap_add.is_empty() {
            "<none>".to_string()
        } else {
            plan.policy.cap_add.join(", ")
        }
    )
    .ok();
    writeln!(
        output,
        "  ports: {}",
        if plan.policy.ports.is_empty() {
            "<none>".to_string()
        } else {
            plan.policy.ports.join(", ")
        }
    )
    .ok();
    writeln!(
        output,
        "  pull_policy: {}",
        plan.policy.pull_policy.as_deref().unwrap_or("<default>")
    )
    .ok();
    writeln!(output).ok();

    writeln!(output, "environment:").ok();
    if plan.environment.variables.is_empty() {
        writeln!(output, "  selected: <none>").ok();
    } else {
        for variable in &plan.environment.variables {
            writeln!(
                output,
                "  - {}={} ({})",
                variable.name,
                variable.value,
                describe_env_source(&variable.source)
            )
            .ok();
        }
    }
    writeln!(
        output,
        "  denied: {}",
        if plan.environment.denied.is_empty() {
            "<none>".to_string()
        } else {
            plan.environment.denied.join(", ")
        }
    )
    .ok();
    writeln!(output).ok();

    writeln!(output, "mounts:").ok();
    for mount in &plan.mounts {
        if mount.kind == "mask" {
            writeln!(output, "  - mask {} (credential masked)", mount.target).ok();
            continue;
        }
        let source = mount
            .source
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<none>".to_string());
        let label = if mount.is_workspace {
            "workspace"
        } else {
            "extra"
        };
        writeln!(
            output,
            "  - {} {} -> {} ({}, {})",
            mount.kind,
            source,
            mount.target,
            if mount.read_only { "ro" } else { "rw" },
            label
        )
        .ok();
    }
    writeln!(output).ok();

    writeln!(output, "caches:").ok();
    if plan.caches.is_empty() {
        writeln!(output, "  <none>").ok();
    } else {
        for cache in &plan.caches {
            writeln!(
                output,
                "  - {} -> {} ({}, source: {})",
                cache.name,
                cache.target,
                if cache.read_only { "ro" } else { "rw" },
                cache.source.as_deref().unwrap_or("<default>")
            )
            .ok();
        }
    }
    writeln!(output).ok();

    writeln!(output, "secrets:").ok();
    if plan.secrets.is_empty() {
        writeln!(output, "  <none>").ok();
    } else {
        for secret in &plan.secrets {
            writeln!(
                output,
                "  - {} -> {} (source: {})",
                secret.name, secret.target, secret.source
            )
            .ok();
        }
    }

    if show_command && let Some(podman_args) = render_podman_command(plan) {
        writeln!(output).ok();
        writeln!(output, "backend command:").ok();
        writeln!(output, "  {podman_args}").ok();
    }

    output
}

fn plan_to_json(
    config_path: &std::path::Path,
    plan: &ExecutionPlan,
    strict_security: bool,
) -> serde_json::Value {
    serde_json::json!({
        "config": config_path.display().to_string(),
        "command": plan.command_string,
        "argv": plan.command,
        "profile": plan.profile_name,
        "mode": format!("{:?}", plan.mode).to_lowercase(),
        "backend": describe_backend(&plan.backend),
        "image": {
            "description": plan.image.description,
            "trust": describe_image_trust(plan.image.trust),
            "verify_signature": plan.image.verify_signature,
        },
        "workspace": {
            "root": plan.workspace.root.display().to_string(),
            "mount": plan.workspace.mount,
            "sandbox_cwd": plan.workspace.sandbox_cwd,
        },
        "policy": {
            "network": plan.policy.network,
            "network_allow": plan.policy.network_allow.iter().map(|(h, ip)| {
                serde_json::json!({"host": h, "ip": ip})
            }).collect::<Vec<_>>(),
            "writable": plan.policy.writable,
            "no_new_privileges": plan.policy.no_new_privileges,
            "read_only_rootfs": plan.policy.read_only_rootfs,
            "reuse_container": plan.policy.reuse_container,
        },
        "environment": {
            "variables": plan.environment.variables.iter().map(|v| {
                serde_json::json!({"name": v.name, "value": v.value, "source": describe_env_source(&v.source)})
            }).collect::<Vec<_>>(),
            "denied": plan.environment.denied,
        },
        "audit": {
            "install_style": plan.audit.install_style,
            "strict_security": strict_security,
            "trusted_image_required": crate::exec::trusted_image_required(plan, strict_security),
            "lockfile": describe_lockfile_audit(&plan.audit.lockfile),
            "pre_run": plan.audit.pre_run.iter().map(|argv| argv.join(" ")).collect::<Vec<_>>(),
        },
        "mounts": plan.mounts.iter().map(|m| serde_json::json!({
            "kind": m.kind,
            "source": m.source.as_ref().map(|p| p.display().to_string()),
            "target": m.target,
            "read_only": m.read_only,
            "workspace": m.is_workspace,
        })).collect::<Vec<_>>(),
    })
}

fn render_inline_audit(result: &crate::audit::InlineAuditResult) -> String {
    use crate::audit::InlineAuditStatus;

    let mut out = String::new();
    writeln!(out).ok();
    writeln!(out, "security-scan:").ok();
    writeln!(out, "  tool: {}", result.tool).ok();

    let status_label = match result.status {
        InlineAuditStatus::Clean => "clean",
        InlineAuditStatus::Findings => "VULNERABILITIES FOUND",
        InlineAuditStatus::ToolNotFound => "tool not installed",
        InlineAuditStatus::Error => "error",
    };
    writeln!(out, "  status: {status_label}").ok();

    if !result.output.trim().is_empty() {
        writeln!(out, "  output: |").ok();
        for line in result.output.trim_end().lines() {
            writeln!(out, "    {line}").ok();
        }
    }

    out
}

fn render_podman_command(plan: &ExecutionPlan) -> Option<String> {
    if !matches!(plan.mode, crate::config::model::ExecutionMode::Sandbox) {
        return None;
    }
    if !matches!(plan.backend, crate::config::BackendKind::Podman) {
        return None;
    }

    let image = match &plan.image.source {
        ResolvedImageSource::Reference(r) => r.clone(),
        ResolvedImageSource::Build { tag, .. } => tag.clone(),
    };

    match crate::backend::podman::build_run_args(plan, &image) {
        Ok(args) => {
            let escaped: Vec<String> = std::iter::once("podman".to_string())
                .chain(args.into_iter().map(|arg| {
                    if arg.contains(' ') || arg.contains(',') {
                        format!("'{arg}'")
                    } else {
                        arg
                    }
                }))
                .collect();
            Some(escaped.join(" "))
        }
        Err(_) => None,
    }
}

fn describe_profile_source(source: &ProfileSource) -> String {
    match source {
        ProfileSource::CliOverride => "cli override".to_string(),
        ProfileSource::ExecSubcommand => "exec subcommand".to_string(),
        ProfileSource::Dispatch { rule_name, pattern } => {
            // pm:<name>:<kind> — generated by package_manager: elaboration
            if let Some(rest) = rule_name.strip_prefix("pm:") {
                let parts: Vec<&str> = rest.splitn(2, ':').collect();
                if parts.len() == 2 {
                    return format!(
                        "package_manager preset `{}` ({}) via pattern `{}`",
                        parts[0], parts[1], pattern
                    );
                }
            }
            format!("dispatch rule `{rule_name}` via pattern `{pattern}`")
        }
        ProfileSource::DefaultProfile => "default profile".to_string(),
        ProfileSource::ImplementationDefault => "implementation default".to_string(),
        ProfileSource::Shadow => "shadow infrastructure".to_string(),
    }
}

fn describe_mode_source(source: &ModeSource) -> &'static str {
    match source {
        ModeSource::CliOverride => "cli override",
        ModeSource::Profile => "profile",
        ModeSource::Default => "default",
    }
}

fn describe_backend(backend: &crate::config::BackendKind) -> &'static str {
    match backend {
        crate::config::BackendKind::Podman => "podman",
        crate::config::BackendKind::Docker => "docker",
    }
}

fn describe_image_trust(trust: crate::resolve::ImageTrust) -> &'static str {
    match trust {
        crate::resolve::ImageTrust::PinnedDigest => "pinned-digest",
        crate::resolve::ImageTrust::MutableReference => "mutable-reference",
        crate::resolve::ImageTrust::LocalBuild => "local-build",
    }
}

fn describe_lockfile_audit(audit: &crate::resolve::LockfileAudit) -> String {
    if !audit.applicable {
        return "not-applicable".to_string();
    }

    if audit.present {
        let requirement = if audit.required {
            "required"
        } else {
            "advisory"
        };
        format!(
            "{requirement}, present ({})",
            audit.expected_files.join(" or ")
        )
    } else {
        let requirement = if audit.required {
            "required"
        } else {
            "advisory"
        };
        format!(
            "{requirement}, missing ({})",
            audit.expected_files.join(" or ")
        )
    }
}

fn describe_network_allow(resolved: &[(String, String)], patterns: &[String]) -> String {
    if resolved.is_empty() && patterns.is_empty() {
        return "<none>".to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    if !resolved.is_empty() {
        let hosts: Vec<String> = {
            let mut seen = Vec::new();
            for (host, _) in resolved {
                if !seen.contains(host) {
                    seen.push(host.clone());
                }
            }
            seen
        };
        parts.push(format!("[resolved] {}", hosts.join(", ")));
    }
    if !patterns.is_empty() {
        parts.push(format!("[patterns] {}", patterns.join(", ")));
    }
    parts.join("; ")
}

fn describe_pre_run(pre_run: &[Vec<String>]) -> String {
    if pre_run.is_empty() {
        return "<none>".to_string();
    }
    pre_run
        .iter()
        .map(|argv| argv.join(" "))
        .collect::<Vec<_>>()
        .join(", ")
}

fn describe_execution_mode(mode: &crate::config::model::ExecutionMode) -> &'static str {
    match mode {
        crate::config::model::ExecutionMode::Host => "host",
        crate::config::model::ExecutionMode::Sandbox => "sandbox",
    }
}

fn describe_cwd_mapping(mapping: &CwdMapping) -> &'static str {
    match mapping {
        CwdMapping::InvocationMapped => "mapped from invocation cwd",
        CwdMapping::WorkspaceRootFallback => "workspace root fallback",
    }
}

fn describe_env_source(source: &EnvVarSource) -> &'static str {
    match source {
        EnvVarSource::PassThrough => "pass-through",
        EnvVarSource::Set => "set",
    }
}

fn describe_user(user: &crate::resolve::ResolvedUser) -> String {
    match user {
        crate::resolve::ResolvedUser::Default => "default".to_string(),
        crate::resolve::ResolvedUser::KeepId => "keep-id".to_string(),
        crate::resolve::ResolvedUser::Explicit { uid, gid } => format!("{uid}:{gid}"),
    }
}
