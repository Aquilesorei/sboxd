use std::fmt::Write as _;
use std::process::ExitCode;

use crate::cli::{Cli, ExplainCommand, OutputFormat};
use crate::config::{LoadOptions, load_config};
use crate::error::SboxError;
use crate::resolve::{ResolutionTarget, ResolvedUser, resolve_execution_plan};

pub fn execute(cli: &Cli, command: &ExplainCommand) -> Result<ExitCode, SboxError> {
    let loaded = load_config(&LoadOptions {
        workspace: cli.workspace.clone(),
        config: cli.config.clone(),
    })?;

    let plan = resolve_execution_plan(cli, &loaded, ResolutionTarget::Plan, &command.command)?;

    let strict = crate::exec::strict_security_enabled(cli, &loaded.config);

    match cli.output_format {
        OutputFormat::Json => {
            let obj = build_json(&plan, strict);
            println!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
        }
        OutputFormat::Text => {
            print!("{}", build_prose(&plan, strict));
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn build_prose(plan: &crate::resolve::ExecutionPlan, strict: bool) -> String {
    let mut out = String::new();

    writeln!(out, "sbox explain: `{}`", plan.command_string).ok();
    writeln!(out, "{}", "─".repeat(60)).ok();
    writeln!(out).ok();

    // ── What will run ─────────────────────────────────────────
    writeln!(out, "WHAT WILL RUN").ok();
    match plan.mode {
        crate::config::model::ExecutionMode::Host => {
            writeln!(
                out,
                "  This command runs directly on your host — no container."
            )
            .ok();
        }
        crate::config::model::ExecutionMode::Sandbox => {
            writeln!(
                out,
                "  This command runs inside a {} container.",
                describe_backend(&plan.backend)
            )
            .ok();
            writeln!(out, "  Image: {}", plan.image.description).ok();
        }
    }
    writeln!(
        out,
        "  Profile: {} ({})",
        plan.profile_name,
        describe_profile_source(&plan.profile_source)
    )
    .ok();
    writeln!(out).ok();

    // ── Network ───────────────────────────────────────────────
    writeln!(out, "NETWORK").ok();
    if plan.policy.network == "off" {
        writeln!(
            out,
            "  Network access is BLOCKED. The container cannot reach the internet."
        )
        .ok();
        writeln!(
            out,
            "  This prevents postinstall scripts from phoning home or exfiltrating data."
        )
        .ok();
    } else if plan.policy.network_allow.is_empty() && plan.policy.network_allow_patterns.is_empty()
    {
        writeln!(
            out,
            "  Network access is ON with no allow-list — full internet access."
        )
        .ok();
        writeln!(
            out,
            "  Consider adding `network_allow` to restrict which hosts are reachable."
        )
        .ok();
    } else {
        let hosts: Vec<&str> = {
            let mut seen = Vec::new();
            for (h, _) in &plan.policy.network_allow {
                if !seen.contains(&h.as_str()) {
                    seen.push(h.as_str());
                }
            }
            seen
        };

        if plan.policy.network_policy == crate::config::model::NetworkPolicy::Firewall {
            writeln!(out, "  Network access is restricted via FIREWALL to:").ok();
        } else {
            writeln!(out, "  Network access is restricted to:").ok();
        }

        for h in &hosts {
            writeln!(out, "    • {h}").ok();
        }
        for p in &plan.policy.network_allow_patterns {
            writeln!(out, "    • {p} (pattern)").ok();
        }

        if plan.policy.network_policy == crate::config::model::NetworkPolicy::Firewall {
            writeln!(
                out,
                "  Egress is strictly enforced at the IP level using nftables inside the container."
            )
            .ok();
            writeln!(out, "  This is a robust hardware-like isolation.").ok();
        } else {
            writeln!(
                out,
                "  Other DNS hostnames are blocked (DNS is disabled and allowed hosts are injected via /etc/hosts)."
            )
            .ok();
            writeln!(
                out,
                "  Note: direct IP connections bypass `network_allow` (it is hostname/DNS-based, not a full egress firewall)."
            )
            .ok();
        }
    }
    writeln!(out).ok();

    // ── Filesystem ────────────────────────────────────────────
    writeln!(out, "FILESYSTEM").ok();

    if let Some(compose) = &plan.compose {
        writeln!(
            out,
            "  This command depends on sidecar services from `{}`.",
            compose.file.display()
        )
        .ok();
        writeln!(
            out,
            "  Services: {}",
            compose.services.join(", ")
        )
        .ok();
        writeln!(out).ok();
    }

    writeln!(
        out,
        "  Your workspace is mounted at {} inside the container.",
        plan.workspace.mount
    )
    .ok();
    if plan.policy.writable {
        writeln!(out, "  The entire workspace is writable.").ok();
    } else {
        let writable: Vec<String> = plan
            .mounts
            .iter()
            .filter(|m| !m.read_only && m.is_workspace && m.kind != "mask")
            .map(|m| m.target.clone())
            .collect();
        if writable.is_empty() {
            writeln!(
                out,
                "  The workspace is READ-ONLY. No files can be modified."
            )
            .ok();
        } else {
            writeln!(out, "  Only these paths are writable:").ok();
            for p in &writable {
                writeln!(out, "    • {p}").ok();
            }
            writeln!(out, "  Everything else is read-only.").ok();
        }
    }
    let masked: Vec<&str> = plan
        .mounts
        .iter()
        .filter(|m| m.kind == "mask")
        .map(|m| m.target.as_str())
        .collect();
    if !masked.is_empty() {
        writeln!(
            out,
            "  These credential files are masked (replaced with /dev/null):"
        )
        .ok();
        for p in &masked {
            writeln!(out, "    • {p}").ok();
        }
    }
    writeln!(out).ok();

    // ── Environment ───────────────────────────────────────────
    writeln!(out, "ENVIRONMENT").ok();
    if plan.environment.variables.is_empty() {
        writeln!(
            out,
            "  No host environment variables are passed into the container."
        )
        .ok();
    } else {
        writeln!(
            out,
            "  {} variable(s) are available inside the container.",
            plan.environment.variables.len()
        )
        .ok();
    }
    if !plan.environment.denied.is_empty() {
        writeln!(
            out,
            "  These variables are explicitly BLOCKED even if set on the host:"
        )
        .ok();
        for v in &plan.environment.denied {
            writeln!(out, "    • {v}").ok();
        }
    }
    writeln!(out).ok();

    // ── Identity ──────────────────────────────────────────────
    writeln!(out, "IDENTITY").ok();
    match &plan.user {
        ResolvedUser::Default | ResolvedUser::KeepId => {
            writeln!(
                out,
                "  The container runs as YOUR user (same UID/GID as the host)."
            )
            .ok();
            writeln!(
                out,
                "  Files created inside the container are owned by you on the host."
            )
            .ok();
        }
        ResolvedUser::Explicit { uid, gid } => {
            writeln!(out, "  The container runs as UID {uid} / GID {gid}.").ok();
        }
    }
    writeln!(out).ok();

    // ── Security summary ─────────────────────────────────────
    writeln!(out, "SECURITY SUMMARY").ok();
    if plan.audit.install_style {
        writeln!(
            out,
            "  This is an INSTALL-STYLE command — highest-risk category."
        )
        .ok();
        writeln!(
            out,
            "  Postinstall scripts run inside the sandbox and cannot:"
        )
        .ok();
        writeln!(
            out,
            "    ✓ read your SSH keys, AWS credentials, or ~/.netrc"
        )
        .ok();
        writeln!(out, "    ✓ reach arbitrary internet hosts").ok();
        writeln!(out, "    ✓ write outside the designated writable paths").ok();
        writeln!(out, "    ✓ exfiltrate tokens blocked by the deny list").ok();
        if plan.policy.no_new_privileges {
            writeln!(
                out,
                "    ✓ gain new privileges (no_new_privileges enforced)"
            )
            .ok();
        }
    } else {
        writeln!(out, "  Standard sandboxed execution.").ok();
    }
    if strict {
        writeln!(out, "  Strict security mode is ON.").ok();
    }
    if crate::exec::trusted_image_required(&plan, strict) {
        writeln!(
            out,
            "  A pinned image digest is required — mutable tags are rejected."
        )
        .ok();
    }

    out
}

fn build_json(plan: &crate::resolve::ExecutionPlan, strict: bool) -> serde_json::Value {
    let writable_paths: Vec<&str> = plan
        .mounts
        .iter()
        .filter(|m| !m.read_only && m.is_workspace && m.kind != "mask")
        .map(|m| m.target.as_str())
        .collect();

    let masked_paths: Vec<&str> = plan
        .mounts
        .iter()
        .filter(|m| m.kind == "mask")
        .map(|m| m.target.as_str())
        .collect();

    let allowed_hosts: Vec<&str> = {
        let mut seen = Vec::new();
        for (h, _) in &plan.policy.network_allow {
            if !seen.contains(&h.as_str()) {
                seen.push(h.as_str());
            }
        }
        seen
    };

    serde_json::json!({
        "command": plan.command_string,
        "profile": plan.profile_name,
        "mode": format!("{:?}", plan.mode).to_lowercase(),
        "image": plan.image.description,
        "network": {
            "policy": plan.policy.network,
            "enforcement": format!("{:?}", plan.policy.network_policy).to_lowercase(),
            "allow": allowed_hosts,
            "allow_patterns": plan.policy.network_allow_patterns,
        },
        "filesystem": {
            "mount": plan.workspace.mount,
            "writable_paths": writable_paths,
            "masked_paths": masked_paths,
        },
        "environment": {
            "variable_count": plan.environment.variables.len(),
            "denied": plan.environment.denied,
        },
        "security": {
            "install_style": plan.audit.install_style,
            "strict": strict,
            "no_new_privileges": plan.policy.no_new_privileges,
            "trusted_image_required": crate::exec::trusted_image_required(plan, strict),
            "firewall": plan.policy.network_policy == crate::config::model::NetworkPolicy::Firewall,
        },
        "compose": plan.compose.as_ref().map(|c| serde_json::json!({
            "file": c.file,
            "services": c.services,
        })),
    })
}

fn describe_backend(backend: &crate::config::BackendKind) -> &'static str {
    match backend {
        crate::config::BackendKind::Podman => "Podman",
        crate::config::BackendKind::Docker => "Docker",
    }
}

fn describe_profile_source(source: &crate::resolve::ProfileSource) -> String {
    match source {
        crate::resolve::ProfileSource::CliOverride => "cli override".to_string(),
        crate::resolve::ProfileSource::ExecSubcommand => "exec subcommand".to_string(),
        crate::resolve::ProfileSource::Dispatch { rule_name, pattern } => {
            format!("matched dispatch rule `{rule_name}` via pattern `{pattern}`")
        }
        crate::resolve::ProfileSource::DefaultProfile => "default profile".to_string(),
        crate::resolve::ProfileSource::ImplementationDefault => {
            "implementation default".to_string()
        }
        crate::resolve::ProfileSource::Shadow => "shadow infrastructure".to_string(),
    }
}
