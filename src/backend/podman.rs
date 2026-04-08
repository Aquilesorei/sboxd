use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use crate::error::SboxError;
use crate::resolve::{
    ExecutionPlan, ResolvedImageSource, ResolvedMount, ResolvedSecret, ResolvedUser,
};

/// Cloud metadata service hostnames blocked when `network_allow` is active.
/// These resolve to 192.0.2.1 (non-routable) inside the container.
/// Does NOT block hardcoded-IP connections (e.g. direct TCP to 169.254.169.254).
pub(crate) const CLOUD_METADATA_HOSTNAMES: &[&str] = &[
    "metadata.google.internal",   // GCP
    "metadata.internal",          // GCP alias
    "instance-data.ec2.internal", // AWS
    "169.254.169.254",            // hostname-style lookup (not the raw IP)
    "100.100.100.200",            // Alibaba Cloud
];

#[derive(Debug, Clone)]
pub(crate) enum SignatureVerificationSupport {
    Available {
        policy: PathBuf,
    },
    Unavailable {
        policy: Option<PathBuf>,
        reason: String,
    },
}

fn inspect_container_pid(container_name: &str) -> Result<i32, SboxError> {
    let output = Command::new("podman")
        .args(["inspect", "-f", "{{.State.Pid}}", container_name])
        .stdin(Stdio::null())
        .output()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    if !output.status.success() {
        return Err(SboxError::BackendCommandFailed {
            backend: "podman".to_string(),
            command: format!("podman inspect -f {{.State.Pid}} {container_name}"),
            status: output.status.code().unwrap_or(1),
        });
    }

    let pid_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let pid: i32 = pid_str.parse().map_err(|_| SboxError::FirewallPolicyUnavailable {
        reason: "could not parse container PID for firewall enforcement".to_string(),
    })?;
    if pid <= 0 {
        return Err(SboxError::FirewallPolicyUnavailable {
            reason: "container PID unavailable for firewall enforcement".to_string(),
        });
    }
    Ok(pid)
}

// ── Pending-output extraction ─────────────────────────────────────────────────
//
// When a writable_path entry points to a file that does not yet exist on the
// host (e.g. uv.lock on a first run), we skip the bind-mount entirely so the
// tool sees no existing file and creates it from scratch.  After the container
// exits we extract the file with `podman cp`, which correctly handles user
// namespace ownership.  This avoids the two failure modes of the old
// temp-file staging approach:
//   1. Tools (uv, cargo, npm) parsing an empty or stub file and crashing.
//   2. Copy-back permission errors when the container user differs from the
//      host user via rootless namespace mapping.

struct PendingOutput {
    /// Container-side path of the file to extract (e.g. "/workspace/uv.lock").
    container_target: String,
    /// Host path where the file should land after extraction.
    final_path: PathBuf,
}

/// Split mounts into (mounts-to-pass-to-podman, pending-outputs-to-cp-after-run).
///
/// Returns minimal valid seed content for a known lockfile format so that the
/// tool can parse-then-overwrite rather than failing on an empty file.
/// `workspace_root` is used to read project metadata (e.g. `requires-python`
/// from `pyproject.toml`) when available.
pub(crate) fn lockfile_seed(path: &std::path::Path, workspace_root: &std::path::Path) -> Vec<u8> {
    match path.file_name().and_then(|n| n.to_str()) {
        Some("uv.lock") => {
            let requires_python = read_requires_python(workspace_root)
                .unwrap_or_else(|| ">=3.8".to_string());
            format!("version = 1\nrequires-python = \"{requires_python}\"\n").into_bytes()
        }
        Some("Cargo.lock") => b"version = 3\n".to_vec(),
        Some("package-lock.json") => b"{\"lockfileVersion\":3}".to_vec(),
        Some("composer.lock") => b"{}".to_vec(),
        _ => Vec::new(),
    }
}

/// Scans `pyproject.toml` in `workspace_root` for a `requires-python` value.
/// Uses a simple line search to avoid pulling in a full TOML parser.
fn read_requires_python(workspace_root: &std::path::Path) -> Option<String> {
    let content = fs::read_to_string(workspace_root.join("pyproject.toml")).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        // Skip comments and non-matching lines.
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("requires-python") {
            let rest = rest.trim().strip_prefix('=')?.trim();
            // Extract the quoted value, ignoring trailing inline comments.
            // Handles: ">=3.13", '>=3.13', >=3.13
            let value = if let Some(inner) = rest
                .strip_prefix('"')
                .and_then(|s| s.split('"').next())
                .or_else(|| rest.strip_prefix('\'').and_then(|s| s.split('\'').next()))
            {
                inner.to_string()
            } else {
                // Unquoted — take up to the first whitespace or comment.
                rest.split_whitespace().next()?.to_string()
            };
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// Pre-creates missing host-side output files (e.g. `uv.lock`) so they can be
/// bind-mounted writably.  A file-level writable bind mount takes precedence over
/// a read-only workspace bind mount, letting the container write through it
/// directly.  Files are seeded with minimal valid content so tools that read
/// before writing don't fail parsing an empty file.
/// Directory mounts are always passed through — creating an empty directory is safe.
fn partition_pending_outputs(
    mounts: &[ResolvedMount],
    workspace_root: &std::path::Path,
) -> (Vec<ResolvedMount>, Vec<PendingOutput>) {
    let mut effective: Vec<ResolvedMount> = Vec::with_capacity(mounts.len());
    let mut pending: Vec<PendingOutput> = Vec::new();

    for mount in mounts {
        // Pre-create missing host-side output files before the pending check.
        // A file-level writable bind mount takes precedence over a read-only
        // workspace bind mount, so the container can write through it directly.
        if mount.kind == "bind" && mount.create && !mount.read_only {
            if let Some(source) = mount.source.as_ref() {
                if source.extension().is_some() && !source.exists() {
                    if let Some(parent) = source.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(source, lockfile_seed(source, workspace_root));
                }
            }
        }

        let is_pending = mount.kind == "bind"
            && mount.create
            && mount.source.as_ref().is_some_and(|s| {
                s.extension().is_some() && !s.exists()
            });

        if is_pending {
            let final_path = mount.source.as_ref().unwrap().clone();
            pending.push(PendingOutput {
                container_target: mount.target.clone(),
                final_path,
            });
            // Intentionally not added to `effective` — no bind-mount for this file.
        } else {
            effective.push(mount.clone());
        }
    }

    (effective, pending)
}

/// Extract pending output files from an exited (but not yet removed) container
/// using `podman cp`.  Silently skips files the tool did not produce (e.g.
/// because the command failed before writing anything).
fn copy_pending_outputs(container_name: &str, pending: &[PendingOutput]) -> Result<(), SboxError> {
    for p in pending {
        let src = format!("{}:{}", container_name, p.container_target);
        let status = Command::new("podman")
            .args(["cp", &src, &p.final_path.display().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|source| SboxError::BackendUnavailable {
                backend: "podman".to_string(),
                source,
            })?;
        // Non-zero exit means the file wasn't in the container (tool didn't create it).
        // Don't surface this as an sbox error — the tool's own exit code is what matters.
        let _ = status;
    }
    Ok(())
}

pub fn execute(plan: &ExecutionPlan) -> Result<ExitCode, SboxError> {
    if plan.policy.reuse_container {
        return execute_via_reusable_session(plan, false);
    }
    run_sandboxed(plan, false)
}

pub fn execute_interactive(plan: &ExecutionPlan) -> Result<ExitCode, SboxError> {
    if plan.policy.reuse_container {
        return execute_via_reusable_session(plan, true);
    }
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    run_sandboxed(plan, tty)
}

fn run_sandboxed(plan: &ExecutionPlan, tty: bool) -> Result<ExitCode, SboxError> {
    let (effective_mounts, pending_outputs) = partition_pending_outputs(&plan.mounts, &plan.workspace.root);

    validate_runtime_inputs_with_mounts(&effective_mounts, &plan.secrets, &plan.workspace.root)?;
    verify_image_signature(plan)?;
    let image = resolve_container_image(plan)?;

    // When there are pending outputs we name the container so we can run
    // `podman cp` after it exits. In firewall mode we also need a container name
    // so we can inspect its PID and apply nftables rules in its network namespace.
    let needs_name_for_firewall = plan.policy.network_policy == crate::config::model::NetworkPolicy::Firewall
        && plan.policy.network != "off"
        && !plan.policy.network_allow.is_empty();
    let container_name = if pending_outputs.is_empty() && !needs_name_for_firewall {
        None
    } else {
        Some(format!("sbox-{}", std::process::id()))
    };

    let args = build_run_args_impl(
        plan,
        &effective_mounts,
        &image,
        tty,
        &plan.command,
        container_name.as_deref(),
    )?;

    let mut child = Command::new("podman");
    child.args(&args);
    child.current_dir(&plan.workspace.effective_host_dir);
    child.stdin(Stdio::inherit());
    child.stdout(Stdio::inherit());
    child.stderr(Stdio::inherit());

    let mut child = child
        .spawn()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    if needs_name_for_firewall {
        let name = container_name.as_deref().expect("named when firewall requested");
        let pid = inspect_container_pid(name)?;
        crate::firewall::apply_firewall_in_netns(
            pid,
            &crate::firewall::FirewallSpec {
                allow_ips: plan.policy.network_allow.iter().map(|(_, ip)| ip.clone()).collect(),
            },
        )?;
    }

    let status = child
        .wait()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    if let Some(name) = &container_name {
        copy_pending_outputs(name, &pending_outputs)?;
        let _ = Command::new("podman")
            .args(["rm", name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    Ok(status_to_exit_code(status))
}

fn execute_via_reusable_session(
    plan: &ExecutionPlan,
    interactive: bool,
) -> Result<ExitCode, SboxError> {
    validate_runtime_inputs(plan)?;
    verify_image_signature(plan)?;
    let image = resolve_container_image(plan)?;
    let session_name = plan
        .policy
        .reusable_session_name
        .as_deref()
        .ok_or_else(|| SboxError::ReusableSandboxSessionsNotImplemented {
            profile: plan.profile_name.clone(),
        })?;

    ensure_reusable_container(plan, &image, session_name)?;

    let tty = interactive && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let mut child = Command::new("podman");
    child.args(build_exec_args(plan, session_name, tty));
    child.current_dir(&plan.workspace.effective_host_dir);
    child.stdin(Stdio::inherit());
    child.stdout(Stdio::inherit());
    child.stderr(Stdio::inherit());

    let status = child
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    Ok(status_to_exit_code(status))
}

fn resolve_container_image(plan: &ExecutionPlan) -> Result<String, SboxError> {
    match &plan.image.source {
        ResolvedImageSource::Reference(reference) => Ok(reference.clone()),
        ResolvedImageSource::Build { recipe_path, tag } => {
            ensure_built_image(recipe_path, tag, &plan.workspace.root)?;
            Ok(tag.clone())
        }
    }
}

fn verify_image_signature(plan: &ExecutionPlan) -> Result<(), SboxError> {
    if !plan.image.verify_signature {
        return Ok(());
    }

    let reference = match &plan.image.source {
        ResolvedImageSource::Reference(reference) => reference.clone(),
        ResolvedImageSource::Build { tag, .. } => {
            return Err(SboxError::SignatureVerificationUnavailable {
                image: tag.clone(),
                reason: "signature verification is not implemented for local build images".into(),
            });
        }
    };

    let policy = match inspect_signature_verification_support()? {
        SignatureVerificationSupport::Available { policy } => policy,
        SignatureVerificationSupport::Unavailable { reason, .. } => {
            return Err(SboxError::SignatureVerificationUnavailable {
                image: reference,
                reason,
            });
        }
    };

    run_signature_verification(&reference, &policy)
}

fn run_signature_verification(reference: &str, policy: &Path) -> Result<(), SboxError> {
    let status = Command::new("skopeo")
        .args([
            "--policy",
            &policy.display().to_string(),
            "inspect",
            "--raw",
            &format!("docker://{reference}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "skopeo".to_string(),
            source,
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(SboxError::SignatureVerificationFailed {
            image: reference.to_string(),
            policy: policy.to_path_buf(),
            reason: format!("skopeo exited with status {}", status.code().unwrap_or(1)),
        })
    }
}

pub(crate) fn inspect_signature_verification_support()
-> Result<SignatureVerificationSupport, SboxError> {
    match Command::new("skopeo")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(_) => {
            return Ok(SignatureVerificationSupport::Unavailable {
                policy: None,
                reason: "skopeo is installed but unusable".into(),
            });
        }
        Err(_) => {
            return Ok(SignatureVerificationSupport::Unavailable {
                policy: None,
                reason: "skopeo is not installed".into(),
            });
        }
    }

    let Some(policy) = resolve_signature_policy_path() else {
        return Ok(SignatureVerificationSupport::Unavailable {
            policy: None,
            reason: "no containers policy file found; configure SBOX_SIGNATURE_POLICY, ~/.config/containers/policy.json, or /etc/containers/policy.json".into(),
        });
    };

    if !policy_supports_signature_verification(&policy)? {
        return Ok(SignatureVerificationSupport::Unavailable {
            policy: Some(policy.clone()),
            reason: "policy does not enforce signature verification".to_string(),
        });
    }

    Ok(SignatureVerificationSupport::Available { policy })
}

fn resolve_signature_policy_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SBOX_SIGNATURE_POLICY") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    if let Some(home) = crate::platform::home_dir() {
        let user_policy = home.join(".config/containers/policy.json");
        if user_policy.is_file() {
            return Some(user_policy);
        }
    }

    // /etc/containers/policy.json is Linux-specific; on other platforms this
    // path won't exist and the check is harmless.
    let system_policy = PathBuf::from("/etc/containers/policy.json");
    system_policy.is_file().then_some(system_policy)
}

fn policy_supports_signature_verification(path: &Path) -> Result<bool, SboxError> {
    let content = fs::read_to_string(path).map_err(|source| SboxError::ConfigRead {
        path: path.to_path_buf(),
        source,
    })?;
    let json: serde_json::Value =
        serde_json::from_str(&content).map_err(|source| SboxError::ConfigValidation {
            message: format!("invalid containers policy {}: {source}", path.display()),
        })?;

    let mut candidates = Vec::new();
    if let Some(default) = json.get("default") {
        candidates.push(default);
    }
    if let Some(transports) = json.get("transports").and_then(|value| value.as_object()) {
        for transport_name in ["docker", "docker-daemon"] {
            if let Some(scopes) = transports
                .get(transport_name)
                .and_then(|value| value.as_object())
            {
                for requirements in scopes.values() {
                    candidates.push(requirements);
                }
            }
        }
    }

    Ok(candidates
        .into_iter()
        .any(requirements_enable_signature_verification))
}

fn requirements_enable_signature_verification(value: &serde_json::Value) -> bool {
    let Some(requirements) = value.as_array() else {
        return false;
    };

    requirements.iter().any(|requirement| {
        requirement
            .get("type")
            .and_then(|value| value.as_str())
            .is_some_and(|kind| matches!(kind, "signedBy" | "sigstoreSigned"))
    })
}

pub fn build_run_args(plan: &ExecutionPlan, image: &str) -> Result<Vec<String>, SboxError> {
    build_run_args_with_options(plan, image, false)
}

pub fn build_run_args_with_options(
    plan: &ExecutionPlan,
    image: &str,
    tty: bool,
) -> Result<Vec<String>, SboxError> {
    build_run_args_impl(plan, &plan.mounts, image, tty, &plan.command, None)
}

fn build_run_args_impl(
    plan: &ExecutionPlan,
    mounts: &[ResolvedMount],
    image: &str,
    tty: bool,
    command: &[String],
    container_name: Option<&str>,
) -> Result<Vec<String>, SboxError> {
    let mut args = vec!["run".to_string()];
    match container_name {
        Some(name) => {
            args.push("--name".to_string());
            args.push(name.to_string());
        }
        None => {
            args.push("--rm".to_string());
        }
    }
    args.push("-i".to_string());

    if tty {
        args.push("-t".to_string());
    }

    args.push("--workdir".to_string());
    args.push(plan.workspace.sandbox_cwd.clone());

    if plan.policy.read_only_rootfs {
        args.push("--read-only".to_string());
    }

    if plan.policy.no_new_privileges {
        args.push("--security-opt".to_string());
        args.push("no-new-privileges".to_string());
    }

    for capability in &plan.policy.cap_drop {
        args.push("--cap-drop".to_string());
        args.push(capability.clone());
    }

    for capability in &plan.policy.cap_add {
        args.push("--cap-add".to_string());
        args.push(capability.clone());
    }

    match plan.policy.network.as_str() {
        "off" => {
            args.push("--network".to_string());
            args.push("none".to_string());
        }
        "on" => {}
        other => {
            args.push("--network".to_string());
            args.push(other.to_string());
        }
    }

    for port in &plan.policy.ports {
        args.push("--publish".to_string());
        args.push(normalize_port_spec(port));
    }

    match &plan.user {
        ResolvedUser::KeepId => {
            args.push("--userns".to_string());
            args.push("keep-id".to_string());
            // Disable SELinux relabeling; rootless keep-id containers cannot set
            // xattrs on device nodes (e.g. /dev/null) when SELinux is enforcing.
            args.push("--security-opt".to_string());
            args.push("label=disable".to_string());
        }
        ResolvedUser::Explicit { uid, gid } => {
            args.push("--user".to_string());
            args.push(format!("{uid}:{gid}"));
        }
        ResolvedUser::Default => {}
    }

    for mount in mounts {
        append_mount_args(&mut args, mount)?;
    }

    for cache in &plan.caches {
        args.push("--mount".to_string());
        if let Some(source) = &cache.source {
            if let Some(path) = try_resolve_host_path(source, &plan.workspace.root) {
                args.push(format!(
                    "type=bind,src={},target={},relabel=private,readonly={}",
                    path.display(),
                    cache.target,
                    bool_string(cache.read_only)
                ));
            } else {
                args.push(format!(
                    "type=volume,src={},target={},readonly={}",
                    source,
                    cache.target,
                    bool_string(cache.read_only)
                ));
            }
        } else {
            args.push(format!(
                "type=volume,src={},target={},readonly={}",
                scoped_cache_name(&plan.workspace.root, &cache.name),
                cache.target,
                bool_string(cache.read_only)
            ));
        }
    }

    for secret in &plan.secrets {
        append_secret_args(&mut args, secret, &plan.workspace.root)?;
    }

    for variable in &plan.environment.variables {
        args.push("--env".to_string());
        args.push(format!("{}={}", variable.name, variable.value));
    }

    if let Some(pull_policy) = &plan.policy.pull_policy {
        args.push("--pull".to_string());
        args.push(pull_policy.clone());
    }

    if let Some((program, rest)) = command.split_first() {
        args.push("--entrypoint".to_string());
        args.push(program.clone());
        args.push(image.to_string());
        args.extend(rest.iter().cloned());
    } else {
        args.push(image.to_string());
    }

    Ok(args)
}

fn append_mount_args(args: &mut Vec<String>, mount: &ResolvedMount) -> Result<(), SboxError> {
    match mount.kind.as_str() {
        "bind" => {
            let source = mount
                .source
                .as_ref()
                .expect("bind mounts always resolve source");
            args.push("--mount".to_string());
            // relabel=private is omitted: we pass --security-opt label=disable for
            // keep-id containers, and relabeling /dev/null (used for mask mounts) would
            // fail with lsetxattr under SELinux + rootless Podman regardless.
            args.push(format!(
                "type=bind,src={},target={},readonly={}",
                source.display(),
                mount.target,
                bool_string(mount.read_only)
            ));
            Ok(())
        }
        "tmpfs" => {
            args.push("--tmpfs".to_string());
            let spec = if mount.read_only {
                format!("{}:ro", mount.target)
            } else {
                mount.target.clone()
            };
            args.push(spec);
            Ok(())
        }
        "mask" => {
            // Overlay the file with /dev/null so the container sees an empty file.
            // The host file is untouched; malicious hooks cannot read its contents.
            // No relabel=private: would trigger lsetxattr on /dev/null under SELinux.
            args.push("--mount".to_string());
            args.push(format!(
                "type=bind,src=/dev/null,target={},readonly=true",
                mount.target
            ));
            Ok(())
        }
        other => Err(SboxError::UnsupportedMountType {
            mount_type: other.to_string(),
        }),
    }
}

fn ensure_reusable_container(
    plan: &ExecutionPlan,
    image: &str,
    session_name: &str,
) -> Result<(), SboxError> {
    match inspect_container_state(session_name)? {
        ContainerState::Running => return Ok(()),
        ContainerState::Stopped => {
            let status = Command::new("podman")
                .args(["start", session_name])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|source| SboxError::BackendUnavailable {
                    backend: "podman".to_string(),
                    source,
                })?;

            if status.success() {
                return Ok(());
            }

            return Err(SboxError::BackendCommandFailed {
                backend: "podman".to_string(),
                command: format!("podman start {session_name}"),
                status: status.code().unwrap_or(1),
            });
        }
        ContainerState::Missing => {}
    }

    let mut create_args = vec![
        "create".to_string(),
        "--name".to_string(),
        session_name.to_string(),
        "--workdir".to_string(),
        plan.workspace.sandbox_cwd.clone(),
        "--entrypoint".to_string(),
        "sleep".to_string(),
    ];
    append_container_settings(&mut create_args, plan)?;
    create_args.push(image.to_string());
    create_args.push("infinity".to_string());

    let create_status = Command::new("podman")
        .args(&create_args)
        .current_dir(&plan.workspace.effective_host_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    if !create_status.success() {
        return Err(SboxError::BackendCommandFailed {
            backend: "podman".to_string(),
            command: format!("podman create --name {session_name} ..."),
            status: create_status.code().unwrap_or(1),
        });
    }

    let start_status = Command::new("podman")
        .args(["start", session_name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    if start_status.success() {
        Ok(())
    } else {
        Err(SboxError::BackendCommandFailed {
            backend: "podman".to_string(),
            command: format!("podman start {session_name}"),
            status: start_status.code().unwrap_or(1),
        })
    }
}

fn build_exec_args(plan: &ExecutionPlan, session_name: &str, tty: bool) -> Vec<String> {
    let mut args = vec!["exec".to_string(), "-i".to_string()];
    if tty {
        args.push("-t".to_string());
    }

    args.push("--workdir".to_string());
    args.push(plan.workspace.sandbox_cwd.clone());

    for variable in &plan.environment.variables {
        args.push("--env".to_string());
        args.push(format!("{}={}", variable.name, variable.value));
    }

    args.push(session_name.to_string());
    args.extend(plan.command.iter().cloned());
    args
}

fn append_container_settings(
    args: &mut Vec<String>,
    plan: &ExecutionPlan,
) -> Result<(), SboxError> {
    if plan.policy.read_only_rootfs {
        args.push("--read-only".to_string());
    }

    if plan.policy.no_new_privileges {
        args.push("--security-opt".to_string());
        args.push("no-new-privileges".to_string());
    }

    for capability in &plan.policy.cap_drop {
        args.push("--cap-drop".to_string());
        args.push(capability.clone());
    }

    for capability in &plan.policy.cap_add {
        args.push("--cap-add".to_string());
        args.push(capability.clone());
    }

    match plan.policy.network.as_str() {
        "off" => {
            args.push("--network".to_string());
            args.push("none".to_string());
        }
        "on" => {}
        other => {
            args.push("--network".to_string());
            args.push(other.to_string());
        }
    }

    // Domain allow-listing (DNS mode only): break default DNS, inject resolved IPs as /etc/hosts entries.
    // RFC 5737 192.0.2.1 is TEST-NET-1, guaranteed non-routable — DNS queries will time out.
    if plan.policy.network_policy != crate::config::model::NetworkPolicy::Firewall
        && !plan.policy.network_allow.is_empty()
    {
        args.push("--dns".to_string());
        args.push("192.0.2.1".to_string());
        for (hostname, ip) in &plan.policy.network_allow {
            args.push("--add-host".to_string());
            args.push(format!("{hostname}:{ip}"));
        }
    }

    // Block cloud metadata service hostnames whenever the network is on, regardless of
    // whether network_allow is set. /etc/hosts entries take priority over DNS.
    // NOTE: scripts connecting directly to 169.254.169.254 by raw IP bypass this;
    // full IP-level blocking requires host-level firewall rules outside of rootless Podman.
    if plan.policy.network != "off" {
        for hostname in CLOUD_METADATA_HOSTNAMES {
            args.push("--add-host".to_string());
            args.push(format!("{hostname}:192.0.2.1"));
        }
    }

    for port in &plan.policy.ports {
        args.push("--publish".to_string());
        // Rootless Podman (pasta) only binds IPv4 by default.  If the user wrote
        // "8000:8000" without a host IP, prepend 127.0.0.1 so that the published
        // port is reachable even when `localhost` resolves to ::1.
        args.push(normalize_port_spec(port));
    }

    match &plan.user {
        ResolvedUser::KeepId => {
            args.push("--userns".to_string());
            args.push("keep-id".to_string());
            args.push("--security-opt".to_string());
            args.push("label=disable".to_string());
        }
        ResolvedUser::Explicit { uid, gid } => {
            args.push("--user".to_string());
            args.push(format!("{uid}:{gid}"));
        }
        ResolvedUser::Default => {}
    }

    for mount in &plan.mounts {
        append_mount_args(args, mount)?;
    }

    for cache in &plan.caches {
        args.push("--mount".to_string());
        if let Some(source) = &cache.source {
            if let Some(path) = try_resolve_host_path(source, &plan.workspace.root) {
                args.push(format!(
                    "type=bind,src={},target={},relabel=private,readonly={}",
                    path.display(),
                    cache.target,
                    bool_string(cache.read_only)
                ));
            } else {
                args.push(format!(
                    "type=volume,src={},target={},readonly={}",
                    source,
                    cache.target,
                    bool_string(cache.read_only)
                ));
            }
        } else {
            args.push(format!(
                "type=volume,src={},target={},readonly={}",
                scoped_cache_name(&plan.workspace.root, &cache.name),
                cache.target,
                bool_string(cache.read_only)
            ));
        }
    }

    for secret in &plan.secrets {
        append_secret_args(args, secret, &plan.workspace.root)?;
    }

    for variable in &plan.environment.variables {
        args.push("--env".to_string());
        args.push(format!("{}={}", variable.name, variable.value));
    }

    Ok(())
}

enum ContainerState {
    Missing,
    Stopped,
    Running,
}

fn inspect_container_state(session_name: &str) -> Result<ContainerState, SboxError> {
    let output = Command::new("podman")
        .args(["inspect", "--format", "{{.State.Running}}", session_name])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim() == "true" {
            Ok(ContainerState::Running)
        } else {
            Ok(ContainerState::Stopped)
        }
    } else if output.status.code() == Some(125) {
        Ok(ContainerState::Missing)
    } else {
        Err(SboxError::BackendCommandFailed {
            backend: "podman".to_string(),
            command: format!("podman inspect {session_name}"),
            status: output.status.code().unwrap_or(1),
        })
    }
}

fn validate_runtime_inputs(plan: &ExecutionPlan) -> Result<(), SboxError> {
    validate_runtime_inputs_with_mounts(&plan.mounts, &plan.secrets, &plan.workspace.root)
}

fn validate_runtime_inputs_with_mounts(
    mounts: &[ResolvedMount],
    secrets: &[ResolvedSecret],
    workspace_root: &Path,
) -> Result<(), SboxError> {
    for mount in mounts {
        validate_mount_source(mount)?;
    }

    for secret in secrets {
        validate_secret_source(secret, workspace_root)?;
    }

    Ok(())
}

fn validate_mount_source(mount: &ResolvedMount) -> Result<(), SboxError> {
    if mount.kind != "bind" {
        // "tmpfs" and "mask" mounts have no host source to validate
        return Ok(());
    }

    let source = mount
        .source
        .as_ref()
        .expect("bind mounts always resolve source");

    if source.exists() {
        return Ok(());
    }

    if mount.create {
        // If the path looks like a file (has an extension), create an empty file.
        // Otherwise create a directory (e.g. node_modules, .cache).
        if source.extension().is_some() {
            if let Some(parent) = source.parent() {
                fs::create_dir_all(parent).ok();
            }
            return fs::write(source, b"").map_err(|_| SboxError::HostPathNotFound {
                kind: "mount source",
                name: mount.target.clone(),
                path: source.clone(),
            });
        }
        return fs::create_dir_all(source).map_err(|_| SboxError::HostPathNotFound {
            kind: "mount source",
            name: mount.target.clone(),
            path: source.clone(),
        });
    }

    Err(SboxError::HostPathNotFound {
        kind: "mount source",
        name: mount.target.clone(),
        path: source.clone(),
    })
}

fn append_secret_args(
    args: &mut Vec<String>,
    secret: &ResolvedSecret,
    workspace_root: &Path,
) -> Result<(), SboxError> {
    let path = validate_secret_source(secret, workspace_root)?;

    args.push("--mount".to_string());
    args.push(format!(
        "type=bind,src={},target={},relabel=private,readonly=true",
        path.display(),
        secret.target
    ));
    Ok(())
}

fn validate_secret_source(
    secret: &ResolvedSecret,
    workspace_root: &Path,
) -> Result<PathBuf, SboxError> {
    let path = try_resolve_host_path(&secret.source, workspace_root).ok_or_else(|| {
        SboxError::UnsupportedSecretSource {
            name: secret.name.clone(),
            secret_source: secret.source.clone(),
        }
    })?;

    if path.exists() {
        Ok(path)
    } else {
        Err(SboxError::HostPathNotFound {
            kind: "secret source",
            name: secret.name.clone(),
            path,
        })
    }
}

fn try_resolve_host_path(input: &str, base: &Path) -> Option<PathBuf> {
    if input.starts_with("~/") || input == "~" {
        let mut path = crate::platform::home_dir()?;
        let remainder = input.strip_prefix("~/").unwrap_or("");
        if !remainder.is_empty() {
            path.push(remainder);
        }
        return Some(path);
    }

    let path = Path::new(input);
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }

    if input.starts_with("./") || input.starts_with("../") || input.contains('/') {
        return Some(base.join(path));
    }

    None
}

fn scoped_cache_name(workspace_root: &Path, cache_name: &str) -> String {
    format!(
        "sbox-cache-{}-{}",
        stable_hash(&workspace_root.display().to_string()),
        sanitize_volume_name(cache_name)
    )
}

fn sanitize_volume_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn stable_hash(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn bool_string(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// Normalises a port spec so it always has an explicit host IP.
/// `"8000:8000"` → `"127.0.0.1:8000:8000"` so that rootless Podman (pasta)
/// binds IPv4 only and `localhost` resolves correctly even on systems where
/// it resolves to `::1` before `127.0.0.1`.
/// Already-qualified specs (`"0.0.0.0:8000:8000"`, `"[::]:8000:8000"`) are
/// returned unchanged.
pub(crate) fn normalize_port_spec(spec: &str) -> String {
    // Count colons: "host:container" has 1, "ip:host:container" has 2+.
    // IPv6 addresses (e.g. [::1]:8000:8000) always start with '['.
    let colon_count = spec.chars().filter(|&c| c == ':').count();
    if colon_count >= 2 || spec.starts_with('[') {
        return spec.to_string();
    }
    format!("127.0.0.1:{spec}")
}

fn ensure_built_image(
    recipe_path: &Path,
    tag: &str,
    workspace_root: &Path,
) -> Result<(), SboxError> {
    let exists_status = Command::new("podman")
        .args(["image", "exists", tag])
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    if exists_status.success() {
        if let Ok(metadata) = fs::metadata(workspace_root.join(recipe_path)) {
            if let Ok(modified) = metadata.modified() {
                let inspect_out = Command::new("podman")
                    .args(["image", "inspect", tag, "--format", "{{.Created}}"])
                    .output();

                if let Ok(out) = inspect_out {
                    let created_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if let Ok(created_dt) = chrono::DateTime::parse_from_rfc3339(&created_str) {
                        if modified > created_dt.into() {
                            println!("Dockerfile `{}` is newer than image `{}`. Rebuilding...", recipe_path.display(), tag);
                        } else {
                            return Ok(());
                        }
                    } else {
                        // Fallback: if we can't parse date, assume it's fine to avoid constant rebuilds
                        return Ok(());
                    }
                } else {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
        } else {
            return Ok(());
        }
    } else if exists_status.code() != Some(1) {
        return Err(SboxError::BackendCommandFailed {
            backend: "podman".to_string(),
            command: format!("podman image exists {tag}"),
            status: exists_status.code().unwrap_or(1),
        });
    }

    let build_args = vec![
        "build".to_string(),
        "-t".to_string(),
        tag.to_string(),
        "-f".to_string(),
        recipe_path.display().to_string(),
        workspace_root.display().to_string(),
    ];

    let build_status = Command::new("podman")
        .args(&build_args)
        .current_dir(workspace_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    if build_status.success() {
        Ok(())
    } else {
        Err(SboxError::BackendCommandFailed {
            backend: "podman".to_string(),
            command: format!(
                "podman build -t {tag} -f {} {}",
                recipe_path.display(),
                workspace_root.display()
            ),
            status: build_status.code().unwrap_or(1),
        })
    }
}

fn status_to_exit_code(status: std::process::ExitStatus) -> ExitCode {
    match status.code() {
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => ExitCode::from(1),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::config::model::ExecutionMode;
    use crate::error::SboxError;
    use crate::resolve::{
        CwdMapping, EnvVarSource, ExecutionPlan, ModeSource, ProfileSource, ResolvedCache,
        ResolvedEnvVar, ResolvedEnvironment, ResolvedImage, ResolvedImageSource, ResolvedMount,
        ResolvedPolicy, ResolvedSecret, ResolvedUser, ResolvedWorkspace,
    };

    use super::{
        build_run_args, build_run_args_with_options, policy_supports_signature_verification,
        requirements_enable_signature_verification, validate_runtime_inputs,
    };

    fn create_temp_fixture() -> (PathBuf, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sbox-podman-test-{unique}"));
        let secret = root.join("secret.txt");
        fs::create_dir_all(&root).expect("temp fixture directory should be created");
        fs::write(&secret, "token").expect("secret fixture should be written");
        (root, secret)
    }

    fn sample_plan() -> ExecutionPlan {
        let (root, secret) = create_temp_fixture();

        ExecutionPlan {
            command: vec!["python".into(), "app.py".into()],
            command_string: "python app.py".into(),
            backend: crate::config::BackendKind::Podman,
            image: ResolvedImage {
                description: "ref:python:3.13-slim".into(),
                source: ResolvedImageSource::Reference("python:3.13-slim".into()),
                trust: crate::resolve::ImageTrust::MutableReference,
                verify_signature: false,
            },
            profile_name: "default".into(),
            profile_source: ProfileSource::DefaultProfile,
            mode: ExecutionMode::Sandbox,
            mode_source: ModeSource::Profile,
            workspace: ResolvedWorkspace {
                root: root.clone(),
                invocation_dir: root.clone(),
                effective_host_dir: root.clone(),
                mount: "/workspace".into(),
                sandbox_cwd: "/workspace".into(),
                cwd_mapping: CwdMapping::InvocationMapped,
            },
            policy: ResolvedPolicy {
                network: "off".into(),
                writable: true,
                ports: vec!["3000:3000".into()],
                no_new_privileges: true,
                read_only_rootfs: false,
                reuse_container: false,
                reusable_session_name: None,
                cap_drop: vec!["all".into()],
                cap_add: Vec::new(),
                pull_policy: None,
                network_allow: Vec::new(),
                network_allow_patterns: Vec::new(),
                network_policy: crate::config::model::NetworkPolicy::Dns,
            },
            environment: ResolvedEnvironment {
                variables: vec![ResolvedEnvVar {
                    name: "APP_ENV".into(),
                    value: "development".into(),
                    source: EnvVarSource::Set,
                }],
                denied: Vec::new(),
            },
            mounts: vec![ResolvedMount {
                kind: "bind".into(),
                source: Some(root.clone()),
                target: "/workspace".into(),
                read_only: false,
                is_workspace: true,
                create: false,
            }],
            caches: vec![ResolvedCache {
                name: "uv-cache".into(),
                target: "/root/.cache/uv".into(),
                source: None,
                read_only: false,
            }],
            secrets: vec![ResolvedSecret {
                name: "token".into(),
                source: secret.display().to_string(),
                target: "/run/secrets/token".into(),
            }],
            user: ResolvedUser::KeepId,
            rootless: true,
            audit: crate::resolve::ExecutionAudit {
                install_style: false,
                trusted_image_required: false,
                sensitive_pass_through_vars: Vec::new(),
                lockfile: crate::resolve::LockfileAudit {
                    applicable: false,
                    required: false,
                    present: false,
                    expected_files: Vec::new(),
                },
                pre_run: Vec::new(),
            },
            compose: None,
        }
    }

    #[test]
    fn builds_expected_podman_arguments() {
        let plan = sample_plan();
        let args = build_run_args(&plan, "python:3.13-slim").expect("builder should succeed");
        let joined = args.join(" ");

        assert!(joined.contains("run --rm -i"));
        assert!(joined.contains("--workdir /workspace"));
        assert!(joined.contains("--network none"));
        assert!(joined.contains("--publish 127.0.0.1:3000:3000"));
        assert!(joined.contains("--userns keep-id"));
        assert!(joined.contains("--entrypoint python"));
        assert!(joined.contains("--cap-drop all"));
        assert!(joined.contains("--env APP_ENV=development"));
        assert!(joined.contains("python:3.13-slim app.py"));
    }

    #[test]
    fn preflight_fails_when_secret_source_is_missing() {
        let mut plan = sample_plan();
        plan.secrets[0].source = "/tmp/definitely-missing-sbox-secret".into();

        let error =
            validate_runtime_inputs(&plan).expect_err("missing secret should fail preflight");
        assert!(matches!(
            error,
            SboxError::HostPathNotFound {
                kind: "secret source",
                ..
            }
        ));
    }

    #[test]
    fn interactive_podman_arguments_enable_tty() {
        let plan = sample_plan();
        let args = build_run_args_with_options(&plan, "python:3.13-slim", true)
            .expect("interactive builder should succeed");

        assert!(args.iter().any(|arg| arg == "-t"));
    }

    #[test]
    fn requirements_detect_signed_by_policy() {
        let requirements = serde_json::json!([
            {
                "type": "signedBy",
                "keyType": "GPGKeys",
                "keyPath": "/tmp/test.pub"
            }
        ]);

        assert!(requirements_enable_signature_verification(&requirements));
    }

    #[test]
    fn insecure_accept_anything_policy_is_not_verifying() {
        let policy_path = write_policy(
            Path::new("/tmp"),
            r#"{
  "default": [
    { "type": "insecureAcceptAnything" }
  ]
}"#,
        );

        assert!(
            !policy_supports_signature_verification(&policy_path)
                .expect("policy inspection should succeed")
        );
        let _ = fs::remove_file(policy_path);
    }

    #[test]
    fn signed_policy_is_detected_as_verifying() {
        let policy_path = write_policy(
            Path::new("/tmp"),
            r#"{
  "default": [
    {
      "type": "signedBy",
      "keyType": "GPGKeys",
      "keyPath": "/tmp/test.pub"
    }
  ]
}"#,
        );

        assert!(
            policy_supports_signature_verification(&policy_path)
                .expect("policy inspection should succeed")
        );
        let _ = fs::remove_file(policy_path);
    }

    fn write_policy(root: &Path, content: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough for tests")
            .as_nanos();
        let path = root.join(format!("sbox-policy-{unique}.json"));
        fs::write(&path, content).expect("policy fixture should be written");
        path
    }
}
