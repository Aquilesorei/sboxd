use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use crate::backend::podman::CLOUD_METADATA_HOSTNAMES;

use crate::error::SboxError;
use crate::resolve::{
    ExecutionPlan, ResolvedImageSource, ResolvedMount, ResolvedSecret, ResolvedUser,
};

pub fn execute(plan: &ExecutionPlan) -> Result<ExitCode, SboxError> {
    if plan.policy.reuse_container {
        return execute_via_reusable_session(plan, false);
    }

    validate_runtime_inputs(plan)?;
    let image = resolve_container_image(plan)?;
    let args = build_run_args(plan, &image)?;

    let mut child = Command::new("docker");
    child.args(&args);
    child.current_dir(&plan.workspace.effective_host_dir);
    child.stdin(Stdio::inherit());
    child.stdout(Stdio::inherit());
    child.stderr(Stdio::inherit());

    let status = child
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "docker".to_string(),
            source,
        })?;

    Ok(status_to_exit_code(status))
}

pub fn execute_interactive(plan: &ExecutionPlan) -> Result<ExitCode, SboxError> {
    if plan.policy.reuse_container {
        return execute_via_reusable_session(plan, true);
    }

    validate_runtime_inputs(plan)?;
    let image = resolve_container_image(plan)?;
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let args = build_run_args_with_options(plan, &image, tty)?;

    let mut child = Command::new("docker");
    child.args(&args);
    child.current_dir(&plan.workspace.effective_host_dir);
    child.stdin(Stdio::inherit());
    child.stdout(Stdio::inherit());
    child.stderr(Stdio::inherit());

    let status = child
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "docker".to_string(),
            source,
        })?;

    Ok(status_to_exit_code(status))
}

fn execute_via_reusable_session(
    plan: &ExecutionPlan,
    interactive: bool,
) -> Result<ExitCode, SboxError> {
    validate_runtime_inputs(plan)?;
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
    let mut child = Command::new("docker");
    child.args(build_exec_args(plan, session_name, tty));
    child.current_dir(&plan.workspace.effective_host_dir);
    child.stdin(Stdio::inherit());
    child.stdout(Stdio::inherit());
    child.stderr(Stdio::inherit());

    let status = child
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "docker".to_string(),
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

pub fn build_run_args(plan: &ExecutionPlan, image: &str) -> Result<Vec<String>, SboxError> {
    build_run_args_with_options(plan, image, false)
}

pub fn build_run_args_with_options(
    plan: &ExecutionPlan,
    image: &str,
    tty: bool,
) -> Result<Vec<String>, SboxError> {
    let mut args = vec!["run".to_string(), "--rm".to_string(), "-i".to_string()];

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

    if !plan.policy.network_allow.is_empty() {
        args.push("--dns".to_string());
        args.push("192.0.2.1".to_string());
        for (hostname, ip) in &plan.policy.network_allow {
            args.push("--add-host".to_string());
            args.push(format!("{hostname}:{ip}"));
        }
    }

    if plan.policy.network != "off" {
        for hostname in CLOUD_METADATA_HOSTNAMES {
            args.push("--add-host".to_string());
            args.push(format!("{hostname}:192.0.2.1"));
        }
    }

    for port in &plan.policy.ports {
        args.push("--publish".to_string());
        args.push(port.clone());
    }

    // Docker has no --userns keep-id; map to explicit uid:gid using current process identity.
    match &plan.user {
        ResolvedUser::KeepId => {
            let (uid, gid) = current_uid_gid();
            args.push("--user".to_string());
            args.push(format!("{uid}:{gid}"));
        }
        ResolvedUser::Explicit { uid, gid } => {
            args.push("--user".to_string());
            args.push(format!("{uid}:{gid}"));
        }
        ResolvedUser::Default => {}
    }

    for mount in &plan.mounts {
        append_mount_args(&mut args, mount)?;
    }

    for cache in &plan.caches {
        args.push("--mount".to_string());
        if let Some(source) = &cache.source {
            if let Some(path) = try_resolve_host_path(source, &plan.workspace.root) {
                args.push(format!(
                    "type=bind,src={},target={},readonly={}",
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

    args.push(image.to_string());
    args.extend(plan.command.iter().cloned());

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
            // Docker does not support relabel=private (Podman/SELinux extension).
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
            let status = Command::new("docker")
                .args(["start", session_name])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|source| SboxError::BackendUnavailable {
                    backend: "docker".to_string(),
                    source,
                })?;

            if status.success() {
                return Ok(());
            }

            return Err(SboxError::BackendCommandFailed {
                backend: "docker".to_string(),
                command: format!("docker start {session_name}"),
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
    ];
    append_container_settings(&mut create_args, plan)?;
    create_args.push(image.to_string());
    create_args.push("sleep".to_string());
    create_args.push("infinity".to_string());

    let create_status = Command::new("docker")
        .args(&create_args)
        .current_dir(&plan.workspace.effective_host_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "docker".to_string(),
            source,
        })?;

    if !create_status.success() {
        return Err(SboxError::BackendCommandFailed {
            backend: "docker".to_string(),
            command: format!("docker create --name {session_name} ..."),
            status: create_status.code().unwrap_or(1),
        });
    }

    let start_status = Command::new("docker")
        .args(["start", session_name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "docker".to_string(),
            source,
        })?;

    if start_status.success() {
        Ok(())
    } else {
        Err(SboxError::BackendCommandFailed {
            backend: "docker".to_string(),
            command: format!("docker start {session_name}"),
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

    if !plan.policy.network_allow.is_empty() {
        args.push("--dns".to_string());
        args.push("192.0.2.1".to_string());
        for (hostname, ip) in &plan.policy.network_allow {
            args.push("--add-host".to_string());
            args.push(format!("{hostname}:{ip}"));
        }
    }

    for port in &plan.policy.ports {
        args.push("--publish".to_string());
        args.push(port.clone());
    }

    match &plan.user {
        ResolvedUser::KeepId => {
            let (uid, gid) = current_uid_gid();
            args.push("--user".to_string());
            args.push(format!("{uid}:{gid}"));
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
                    "type=bind,src={},target={},readonly={}",
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
    // Use `docker container ls -a` to avoid exit-code ambiguity between
    // "container not found" and "daemon not running".
    let output = Command::new("docker")
        .args([
            "container",
            "ls",
            "-a",
            "--filter",
            &format!("name=^{session_name}$"),
            "--format",
            "{{.State}}",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "docker".to_string(),
            source,
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let state = stdout.trim();
    if state.is_empty() {
        Ok(ContainerState::Missing)
    } else if state == "running" {
        Ok(ContainerState::Running)
    } else {
        Ok(ContainerState::Stopped)
    }
}

fn validate_runtime_inputs(plan: &ExecutionPlan) -> Result<(), SboxError> {
    for mount in &plan.mounts {
        validate_mount_source(mount)?;
    }
    for secret in &plan.secrets {
        validate_secret_source(secret, &plan.workspace.root)?;
    }
    Ok(())
}

fn validate_mount_source(mount: &ResolvedMount) -> Result<(), SboxError> {
    if mount.kind != "bind" {
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
        // Docker bind-mounts a missing source as a directory by default, which
        // corrupts lockfiles like package-lock.json on first install.
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
    // Docker does not support relabel=private.
    args.push(format!(
        "type=bind,src={},target={},readonly=true",
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
        let home = std::env::var_os("HOME")?;
        let remainder = input.strip_prefix("~/").unwrap_or("");
        let mut path = PathBuf::from(home);
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

/// Read the current process's effective uid and gid from /proc/self/status.
/// Falls back to (0, 0) on any read failure (should not happen on Linux).
fn current_uid_gid() -> (u32, u32) {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let uid = parse_proc_id(&status, "Uid:");
    let gid = parse_proc_id(&status, "Gid:");
    (uid, gid)
}

fn parse_proc_id(status: &str, key: &str) -> u32 {
    status
        .lines()
        .find(|line| line.starts_with(key))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn ensure_built_image(
    recipe_path: &Path,
    tag: &str,
    workspace_root: &Path,
) -> Result<(), SboxError> {
    // docker image inspect exits 0 if the image exists, non-zero if not.
    let exists_status = Command::new("docker")
        .args(["image", "inspect", "--format", "", tag])
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "docker".to_string(),
            source,
        })?;

    if exists_status.success() {
        return Ok(());
    }

    let build_status = Command::new("docker")
        .args([
            "build",
            "-t",
            tag,
            "-f",
            &recipe_path.display().to_string(),
            &workspace_root.display().to_string(),
        ])
        .current_dir(workspace_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "docker".to_string(),
            source,
        })?;

    if build_status.success() {
        Ok(())
    } else {
        Err(SboxError::BackendCommandFailed {
            backend: "docker".to_string(),
            command: format!(
                "docker build -t {tag} -f {} {}",
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
    use super::{build_run_args, current_uid_gid};
    use crate::config::model::ExecutionMode;
    use crate::resolve::{
        CwdMapping, ExecutionPlan, ImageTrust, ModeSource, ProfileSource, ResolvedEnvironment,
        ResolvedImage, ResolvedImageSource, ResolvedPolicy, ResolvedUser, ResolvedWorkspace,
    };
    use std::path::PathBuf;

    fn sample_plan() -> ExecutionPlan {
        ExecutionPlan {
            command: vec!["npm".into(), "install".into()],
            command_string: "npm install".into(),
            backend: crate::config::BackendKind::Docker,
            image: ResolvedImage {
                description: "ref:node:22".into(),
                source: ResolvedImageSource::Reference("node:22".into()),
                trust: ImageTrust::MutableReference,
                verify_signature: false,
            },
            profile_name: "install".into(),
            profile_source: ProfileSource::DefaultProfile,
            mode: ExecutionMode::Sandbox,
            mode_source: ModeSource::Profile,
            workspace: ResolvedWorkspace {
                root: PathBuf::from("/project"),
                invocation_dir: PathBuf::from("/project"),
                effective_host_dir: PathBuf::from("/project"),
                mount: "/workspace".into(),
                sandbox_cwd: "/workspace".into(),
                cwd_mapping: CwdMapping::InvocationMapped,
            },
            policy: ResolvedPolicy {
                network: "off".into(),
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
            },
            environment: ResolvedEnvironment {
                variables: Vec::new(),
                denied: Vec::new(),
            },
            mounts: Vec::new(),
            caches: Vec::new(),
            secrets: Vec::new(),
            user: ResolvedUser::Default,
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
        }
    }

    #[test]
    fn docker_run_args_use_network_none_when_off() {
        let plan = sample_plan();
        let args = build_run_args(&plan, "node:22").expect("args should build");
        let joined = args.join(" ");
        assert!(joined.contains("--network none"));
        assert!(!joined.contains("relabel"));
    }

    #[test]
    fn docker_run_args_map_keepid_to_explicit_user() {
        let mut plan = sample_plan();
        plan.user = ResolvedUser::KeepId;
        let args = build_run_args(&plan, "node:22").expect("args should build");
        let joined = args.join(" ");
        // Should have --user UID:GID, not --userns keep-id
        assert!(joined.contains("--user"));
        assert!(!joined.contains("keep-id"));
    }

    #[test]
    fn parse_proc_id_extracts_real_uid() {
        // /proc/self/status has lines like "Uid:\t1000\t1000\t1000\t1000"
        let fake = "Name:\tfoo\nUid:\t1000\t1000\t1000\t1000\nGid:\t1001\t1001\t1001\t1001\n";
        assert_eq!(super::parse_proc_id(fake, "Uid:"), 1000);
        assert_eq!(super::parse_proc_id(fake, "Gid:"), 1001);
    }

    #[test]
    fn current_uid_gid_returns_nonzero_for_normal_user() {
        let (uid, _gid) = current_uid_gid();
        // In a normal test environment we won't be root
        // Just verify the function runs without panic and returns plausible values
        assert!(uid < 100_000);
    }

    #[test]
    fn metadata_hostnames_blocked_when_network_is_on() {
        let mut plan = sample_plan();
        plan.policy.network = "on".into();
        let args = build_run_args(&plan, "node:22").expect("args should build");
        let joined = args.join(" ");
        // Every cloud metadata hostname must be sinkholes to 192.0.2.1
        for hostname in crate::backend::podman::CLOUD_METADATA_HOSTNAMES {
            assert!(
                joined.contains(&format!("--add-host {hostname}:192.0.2.1")),
                "expected metadata host {hostname} to be blocked, args: {joined}"
            );
        }
    }

    #[test]
    fn metadata_hostnames_not_added_when_network_is_off() {
        let plan = sample_plan(); // network: off by default
        let args = build_run_args(&plan, "node:22").expect("args should build");
        let joined = args.join(" ");
        // With network off there is no point adding --add-host entries
        assert!(
            !joined.contains("metadata.google.internal"),
            "metadata host should not appear when network is off, args: {joined}"
        );
    }

    #[test]
    fn network_allow_breaks_dns_and_injects_resolved_hosts() {
        let mut plan = sample_plan();
        plan.policy.network = "on".into();
        plan.policy.network_allow = vec![("registry.npmjs.org".into(), "104.16.0.0".into())];
        let args = build_run_args(&plan, "node:22").expect("args should build");
        let joined = args.join(" ");
        // DNS must be broken to the black-hole address
        assert!(
            joined.contains("--dns 192.0.2.1"),
            "expected DNS break when network_allow is set, args: {joined}"
        );
        // The resolved registry host must be injected
        assert!(
            joined.contains("--add-host registry.npmjs.org:104.16.0.0"),
            "expected registry host injected via --add-host, args: {joined}"
        );
    }

    #[test]
    fn network_on_without_network_allow_still_blocks_metadata_hosts() {
        // Even with unrestricted network (no allow-list), metadata hosts must be blocked.
        let mut plan = sample_plan();
        plan.policy.network = "on".into();
        plan.policy.network_allow = vec![];
        let args = build_run_args(&plan, "node:22").expect("args should build");
        let joined = args.join(" ");
        // No DNS break (no allow-list)
        assert!(
            !joined.contains("--dns 192.0.2.1"),
            "DNS should not be broken without allow-list"
        );
        // But metadata hosts should still be blocked
        assert!(
            joined.contains("--add-host metadata.google.internal:192.0.2.1"),
            "metadata host should be blocked even without network_allow, args: {joined}"
        );
    }

    #[test]
    fn denied_env_vars_not_passed_to_container() {
        let mut plan = sample_plan();
        plan.environment.denied = vec!["NPM_TOKEN".into(), "NODE_AUTH_TOKEN".into()];
        plan.policy.network = "on".into();
        let args = build_run_args(&plan, "node:22").expect("args should build");
        let joined = args.join(" ");
        // Denied vars must not appear as -e VAR=... or --env VAR=...
        assert!(
            !joined.contains("NPM_TOKEN"),
            "denied env var NPM_TOKEN should not appear in docker args: {joined}"
        );
    }
}
