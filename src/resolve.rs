use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::cli::{Cli, CliBackendKind, CliExecutionMode};
use crate::config::{
    BackendKind, ImageConfig, LoadedConfig,
    model::{
        AuditHook, CacheConfig, Config, EnvironmentConfig, ExecutionMode, MountType, ProfileConfig,
        ScriptPolicy, SecretConfig,
    },
};
use crate::dispatch;
use crate::error::SboxError;

#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    pub command: Vec<String>,
    pub command_string: String,
    pub backend: BackendKind,
    pub image: ResolvedImage,
    pub profile_name: String,
    pub profile_source: ProfileSource,
    pub mode: ExecutionMode,
    pub mode_source: ModeSource,
    pub workspace: ResolvedWorkspace,
    pub policy: ResolvedPolicy,
    pub environment: ResolvedEnvironment,
    pub mounts: Vec<ResolvedMount>,
    pub caches: Vec<ResolvedCache>,
    pub secrets: Vec<ResolvedSecret>,
    pub user: ResolvedUser,
    pub audit: ExecutionAudit,
}

#[derive(Debug, Clone)]
pub struct ExecutionAudit {
    pub install_style: bool,
    pub trusted_image_required: bool,
    pub sensitive_pass_through_vars: Vec<String>,
    pub package_manager: Option<String>,
    pub lockfile: LockfileAudit,
    pub script_hooks: ScriptHookAudit,
    pub audit_hooks: AuditHookAudit,
}

#[derive(Debug, Clone)]
pub struct LockfileAudit {
    pub applicable: bool,
    pub required: bool,
    pub present: bool,
    pub expected_files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ScriptHookAudit {
    pub applicable: bool,
    pub blocked: bool,
    pub policy: ScriptPolicyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptPolicyState {
    Allow,
    Ignore,
    Block,
}

#[derive(Debug, Clone)]
pub struct AuditHookAudit {
    pub configured: Vec<String>,
    pub runnable: Vec<AuditHookExecution>,
}

#[derive(Debug, Clone)]
pub struct AuditHookExecution {
    pub name: String,
    pub command: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedImage {
    pub description: String,
    pub source: ResolvedImageSource,
    pub trust: ImageTrust,
    pub verify_signature: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ResolvedImageSource {
    Reference(String),
    Build { recipe_path: PathBuf, tag: String },
    Preset(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageTrust {
    PinnedDigest,
    MutableReference,
    LocalBuild,
}

#[derive(Debug, Clone)]
pub struct ResolvedWorkspace {
    pub root: PathBuf,
    pub invocation_dir: PathBuf,
    pub effective_host_dir: PathBuf,
    pub mount: String,
    pub sandbox_cwd: String,
    pub cwd_mapping: CwdMapping,
}

#[derive(Debug, Clone)]
pub enum CwdMapping {
    InvocationMapped,
    WorkspaceRootFallback,
}

#[derive(Debug, Clone)]
pub struct ResolvedPolicy {
    pub network: String,
    pub writable: bool,
    pub ports: Vec<String>,
    pub no_new_privileges: bool,
    pub read_only_rootfs: bool,
    pub reuse_container: bool,
    pub reusable_session_name: Option<String>,
    pub cap_drop: Vec<String>,
    pub cap_add: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedEnvironment {
    pub variables: Vec<ResolvedEnvVar>,
    pub denied: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedEnvVar {
    pub name: String,
    pub value: String,
    pub source: EnvVarSource,
}

#[derive(Debug, Clone)]
pub enum EnvVarSource {
    PassThrough,
    Set,
}

#[derive(Debug, Clone)]
pub struct ResolvedMount {
    pub kind: String,
    pub source: Option<PathBuf>,
    pub target: String,
    pub read_only: bool,
    pub is_workspace: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedCache {
    pub name: String,
    pub target: String,
    pub source: Option<String>,
    pub read_only: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedSecret {
    pub name: String,
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone)]
pub enum ResolvedUser {
    Default,
    KeepId,
    Explicit { uid: u32, gid: u32 },
}

#[derive(Debug, Clone)]
pub enum ProfileSource {
    CliOverride,
    ExecSubcommand,
    Dispatch { rule_name: String, pattern: String },
    DefaultProfile,
    ImplementationDefault,
}

#[derive(Debug, Clone)]
pub enum ModeSource {
    CliOverride,
    Profile,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum ResolutionTarget<'a> {
    Run,
    Exec { profile: &'a str },
    Shell,
    Plan,
}

pub fn resolve_execution_plan(
    cli: &Cli,
    loaded: &LoadedConfig,
    target: ResolutionTarget<'_>,
    command: &[String],
) -> Result<ExecutionPlan, SboxError> {
    let config = &loaded.config;
    let workspace = config.workspace.as_ref().expect("validated workspace");
    let environment = config.environment.as_ref().cloned().unwrap_or_default();
    let profile_resolution = resolve_profile(cli, config, target, command)?;
    let profile = config
        .profiles
        .get(&profile_resolution.name)
        .expect("profile existence validated during resolution");
    let (mode, mode_source) = resolve_mode(cli, profile);
    let backend = resolve_backend(cli, config);
    let resolved_workspace = resolve_workspace(
        loaded,
        workspace
            .mount
            .as_deref()
            .expect("validated workspace mount"),
    );
    let image = resolve_image(
        cli,
        config.image.as_ref().expect("validated image"),
        profile.image.as_ref(),
        &loaded.workspace_root,
    )?;
    let policy = resolve_policy(
        config,
        &profile_resolution.name,
        profile,
        &mode,
        &resolved_workspace.root,
    );
    let environment = resolve_environment(&environment);
    let mounts = resolve_mounts(
        config,
        &resolved_workspace.root,
        &resolved_workspace.mount,
        policy.writable,
    );
    let caches = resolve_caches(&config.caches);
    let secrets = resolve_secrets(&config.secrets, &profile_resolution.name);
    let user = resolve_user(config);
    let audit = ExecutionAudit {
        install_style: is_install_style_command(command, &profile_resolution.name),
        trusted_image_required: profile.require_pinned_image.unwrap_or(false),
        sensitive_pass_through_vars: resolved_sensitive_pass_through_vars(&environment),
        package_manager: detect_package_manager(command).map(str::to_string),
        lockfile: resolve_lockfile_audit(
            command,
            &resolved_workspace.effective_host_dir,
            profile.require_lockfile,
        ),
        script_hooks: resolve_script_hook_audit(
            command,
            &environment,
            profile.script_policy.clone(),
        ),
        audit_hooks: resolve_audit_hook_audit(command, &profile.audit_hooks),
    };

    Ok(ExecutionPlan {
        command: command.to_vec(),
        command_string: dispatch::command_string(command),
        backend,
        image,
        profile_name: profile_resolution.name,
        profile_source: profile_resolution.source,
        mode,
        mode_source,
        workspace: resolved_workspace,
        policy,
        environment,
        mounts,
        caches,
        secrets,
        user,
        audit,
    })
}

struct ProfileResolution {
    name: String,
    source: ProfileSource,
}

fn resolve_profile(
    cli: &Cli,
    config: &Config,
    target: ResolutionTarget<'_>,
    command: &[String],
) -> Result<ProfileResolution, SboxError> {
    if let Some(name) = &cli.profile {
        return ensure_profile_exists(config, name, ProfileSource::CliOverride);
    }

    if let ResolutionTarget::Exec { profile } = target {
        return ensure_profile_exists(config, profile, ProfileSource::ExecSubcommand);
    }

    if matches!(target, ResolutionTarget::Shell) {
        if config.profiles.contains_key("default") {
            return ensure_profile_exists(config, "default", ProfileSource::DefaultProfile);
        }

        if let Some((name, _)) = config.profiles.first() {
            return ensure_profile_exists(config, name, ProfileSource::ImplementationDefault);
        }

        return Err(SboxError::ProfileResolutionFailed {
            command: "<shell>".to_string(),
        });
    }

    let command_string = dispatch::command_string(command);
    for (rule_name, rule) in &config.dispatch {
        for pattern in &rule.patterns {
            if dispatch::matches(pattern, &command_string) {
                return ensure_profile_exists(
                    config,
                    &rule.profile,
                    ProfileSource::Dispatch {
                        rule_name: rule_name.clone(),
                        pattern: pattern.clone(),
                    },
                );
            }
        }
    }

    if config.profiles.contains_key("default") {
        return ensure_profile_exists(config, "default", ProfileSource::DefaultProfile);
    }

    if let Some((name, _)) = config.profiles.first() {
        return ensure_profile_exists(config, name, ProfileSource::ImplementationDefault);
    }

    Err(SboxError::ProfileResolutionFailed {
        command: command_string,
    })
}

fn ensure_profile_exists(
    config: &Config,
    name: &str,
    source: ProfileSource,
) -> Result<ProfileResolution, SboxError> {
    if config.profiles.contains_key(name) {
        Ok(ProfileResolution {
            name: name.to_string(),
            source,
        })
    } else {
        Err(SboxError::UnknownProfile {
            name: name.to_string(),
        })
    }
}

fn resolve_mode(cli: &Cli, profile: &ProfileConfig) -> (ExecutionMode, ModeSource) {
    match cli.mode {
        Some(CliExecutionMode::Host) => (ExecutionMode::Host, ModeSource::CliOverride),
        Some(CliExecutionMode::Sandbox) => (ExecutionMode::Sandbox, ModeSource::CliOverride),
        None => (profile.mode.clone(), ModeSource::Profile),
    }
}

fn resolve_backend(cli: &Cli, config: &Config) -> BackendKind {
    match cli.backend {
        Some(CliBackendKind::Podman) => BackendKind::Podman,
        Some(CliBackendKind::Docker) => BackendKind::Docker,
        None => config
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.backend.clone())
            .expect("validated backend"),
    }
}

fn resolve_image(
    cli: &Cli,
    image: &ImageConfig,
    profile_image: Option<&ImageConfig>,
    workspace_root: &Path,
) -> Result<ResolvedImage, SboxError> {
    if let Some(reference) = &cli.image {
        return Ok(ResolvedImage {
            description: format!("ref:{reference} (cli override)"),
            source: ResolvedImageSource::Reference(reference.clone()),
            trust: classify_reference_trust(reference, None),
            verify_signature: false,
        });
    }

    if let Some(image) = profile_image {
        if let Some(reference) = &image.reference {
            let resolved_reference = attach_digest(reference, image.digest.as_deref());
            return Ok(ResolvedImage {
                description: format!("ref:{resolved_reference} (profile override)"),
                source: ResolvedImageSource::Reference(resolved_reference.clone()),
                trust: classify_reference_trust(&resolved_reference, image.digest.as_deref()),
                verify_signature: image.verify_signature.unwrap_or(false),
            });
        }

        if let Some(build) = &image.build {
            let recipe_path = resolve_relative_path(build, workspace_root);
            let tag = image.tag.clone().unwrap_or_else(|| {
                format!(
                    "sbox-build-{}",
                    stable_hash(&recipe_path.display().to_string())
                )
            });

            return Ok(ResolvedImage {
                description: format!("build:{} (profile override)", recipe_path.display()),
                source: ResolvedImageSource::Build { recipe_path, tag },
                trust: ImageTrust::LocalBuild,
                verify_signature: image.verify_signature.unwrap_or(false),
            });
        }

        if let Some(preset) = &image.preset {
            let reference = resolve_preset_reference(preset)?;
            let resolved_reference = attach_digest(&reference, image.digest.as_deref());
            return Ok(ResolvedImage {
                description: format!(
                    "preset:{preset} -> ref:{resolved_reference} (profile override)"
                ),
                source: ResolvedImageSource::Reference(resolved_reference.clone()),
                trust: classify_reference_trust(&resolved_reference, image.digest.as_deref()),
                verify_signature: image.verify_signature.unwrap_or(false),
            });
        }
    }

    if let Some(reference) = &image.reference {
        let resolved_reference = attach_digest(reference, image.digest.as_deref());
        return Ok(ResolvedImage {
            description: format!("ref:{resolved_reference}"),
            source: ResolvedImageSource::Reference(resolved_reference.clone()),
            trust: classify_reference_trust(&resolved_reference, image.digest.as_deref()),
            verify_signature: image.verify_signature.unwrap_or(false),
        });
    }

    if let Some(build) = &image.build {
        let recipe_path = resolve_relative_path(build, workspace_root);
        let tag = image.tag.clone().unwrap_or_else(|| {
            format!(
                "sbox-build-{}",
                stable_hash(&recipe_path.display().to_string())
            )
        });

        return Ok(ResolvedImage {
            description: format!("build:{}", recipe_path.display()),
            source: ResolvedImageSource::Build { recipe_path, tag },
            trust: ImageTrust::LocalBuild,
            verify_signature: image.verify_signature.unwrap_or(false),
        });
    }

    if let Some(preset) = &image.preset {
        let reference = resolve_preset_reference(preset)?;
        let resolved_reference = attach_digest(&reference, image.digest.as_deref());
        return Ok(ResolvedImage {
            description: format!("preset:{preset} -> ref:{resolved_reference}"),
            source: ResolvedImageSource::Reference(resolved_reference.clone()),
            trust: classify_reference_trust(&resolved_reference, image.digest.as_deref()),
            verify_signature: image.verify_signature.unwrap_or(false),
        });
    }

    Ok(ResolvedImage {
        description: "<missing>".to_string(),
        source: ResolvedImageSource::Preset("<missing>".to_string()),
        trust: ImageTrust::MutableReference,
        verify_signature: false,
    })
}

fn resolve_workspace(loaded: &LoadedConfig, mount: &str) -> ResolvedWorkspace {
    if let Ok(relative) = loaded.invocation_dir.strip_prefix(&loaded.workspace_root) {
        let sandbox_cwd = join_sandbox_path(mount, relative);
        ResolvedWorkspace {
            root: loaded.workspace_root.clone(),
            invocation_dir: loaded.invocation_dir.clone(),
            effective_host_dir: loaded.invocation_dir.clone(),
            mount: mount.to_string(),
            sandbox_cwd,
            cwd_mapping: CwdMapping::InvocationMapped,
        }
    } else {
        ResolvedWorkspace {
            root: loaded.workspace_root.clone(),
            invocation_dir: loaded.invocation_dir.clone(),
            effective_host_dir: loaded.workspace_root.clone(),
            mount: mount.to_string(),
            sandbox_cwd: mount.to_string(),
            cwd_mapping: CwdMapping::WorkspaceRootFallback,
        }
    }
}

fn resolve_policy(
    config: &Config,
    profile_name: &str,
    profile: &ProfileConfig,
    mode: &ExecutionMode,
    workspace_root: &Path,
) -> ResolvedPolicy {
    let (cap_drop, cap_add) = resolve_capabilities(profile);
    let reuse_container = profile.reuse_container.unwrap_or_else(|| {
        config
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.reuse_container)
            .unwrap_or(false)
    });

    ResolvedPolicy {
        network: profile.network.clone().unwrap_or_else(|| "off".to_string()),
        writable: profile.writable.unwrap_or(true),
        ports: if matches!(mode, ExecutionMode::Sandbox) {
            profile.ports.clone()
        } else {
            Vec::new()
        },
        no_new_privileges: profile.no_new_privileges.unwrap_or(true),
        read_only_rootfs: profile.read_only_rootfs.unwrap_or(false),
        reuse_container,
        reusable_session_name: reuse_container
            .then(|| reusable_session_name(config, workspace_root, profile_name)),
        cap_drop,
        cap_add,
    }
}

fn resolve_capabilities(profile: &ProfileConfig) -> (Vec<String>, Vec<String>) {
    match &profile.capabilities {
        Some(crate::config::model::CapabilitiesSpec::Keyword(keyword)) if keyword == "drop-all" => {
            (vec!["all".to_string()], Vec::new())
        }
        Some(crate::config::model::CapabilitiesSpec::List(values)) => (Vec::new(), values.clone()),
        Some(crate::config::model::CapabilitiesSpec::Keyword(keyword)) => {
            (Vec::new(), vec![keyword.clone()])
        }
        None => (Vec::new(), Vec::new()),
    }
}

fn resolve_environment(config: &EnvironmentConfig) -> ResolvedEnvironment {
    let denied: BTreeSet<&str> = config.deny.iter().map(String::as_str).collect();
    let mut variables = BTreeMap::<String, ResolvedEnvVar>::new();

    for name in &config.pass_through {
        if denied.contains(name.as_str()) {
            continue;
        }

        if let Ok(value) = std::env::var(name) {
            variables.insert(
                name.clone(),
                ResolvedEnvVar {
                    name: name.clone(),
                    value,
                    source: EnvVarSource::PassThrough,
                },
            );
        }
    }

    for (name, value) in &config.set {
        variables.insert(
            name.clone(),
            ResolvedEnvVar {
                name: name.clone(),
                value: value.clone(),
                source: EnvVarSource::Set,
            },
        );
    }

    ResolvedEnvironment {
        variables: variables.into_values().collect(),
        denied: config.deny.clone(),
    }
}

fn resolved_sensitive_pass_through_vars(environment: &ResolvedEnvironment) -> Vec<String> {
    environment
        .variables
        .iter()
        .filter(|variable| {
            matches!(variable.source, EnvVarSource::PassThrough)
                && looks_like_sensitive_env(&variable.name)
        })
        .map(|variable| variable.name.clone())
        .collect()
}

fn is_install_style_command(command: &[String], profile_name: &str) -> bool {
    let joined = command.join(" ");
    let command_match = [
        "npm install",
        "npm ci",
        "npm update",
        "npm prune",
        "npm rebuild",
        "pnpm install",
        "pnpm add",
        "pnpm update",
        "pnpm up",
        "pnpm fetch",
        "yarn install",
        "yarn add",
        "yarn up",
        "yarn remove",
        "bun install",
        "bun add",
        "bun update",
        "pip install",
        "uv sync",
        "poetry install",
        "poetry sync",
        "poetry add",
        "poetry update",
        "cargo install",
        "go get",
        "go install",
        "go work sync",
        "go install",
        "composer install",
        "composer update",
        "composer require",
    ]
    .iter()
    .any(|pattern| joined.starts_with(pattern));

    let profile_match = matches!(
        profile_name,
        "install" | "deps" | "dependency-install" | "bootstrap"
    ) || profile_name.contains("install");

    command_match || profile_match
}

fn looks_like_sensitive_env(name: &str) -> bool {
    const EXACT: &[&str] = &[
        "SSH_AUTH_SOCK",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "NPM_TOKEN",
        "NODE_AUTH_TOKEN",
        "PYPI_TOKEN",
        "DOCKER_CONFIG",
        "KUBECONFIG",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "AZURE_CLIENT_SECRET",
        "AWS_SESSION_TOKEN",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_ACCESS_KEY_ID",
    ];
    const PREFIXES: &[&str] = &["AWS_", "GCP_", "GOOGLE_", "AZURE_", "CLOUDSDK_"];

    EXACT.contains(&name) || PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

fn resolve_mounts(
    config: &Config,
    workspace_root: &Path,
    workspace_mount: &str,
    profile_writable: bool,
) -> Vec<ResolvedMount> {
    let workspace_writable = config
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.writable)
        .unwrap_or(true)
        && profile_writable;

    let mut mounts = vec![ResolvedMount {
        kind: "bind".to_string(),
        source: Some(workspace_root.to_path_buf()),
        target: workspace_mount.to_string(),
        read_only: !workspace_writable,
        is_workspace: true,
    }];

    for mount in &config.mounts {
        let source = match mount.mount_type {
            MountType::Bind => mount
                .source
                .as_deref()
                .map(|path| resolve_relative_path(path, workspace_root)),
            MountType::Tmpfs => None,
        };

        mounts.push(ResolvedMount {
            kind: match mount.mount_type {
                MountType::Bind => "bind".to_string(),
                MountType::Tmpfs => "tmpfs".to_string(),
            },
            source,
            target: mount.target.clone().expect("validated mount target"),
            read_only: mount.read_only.unwrap_or(false),
            is_workspace: false,
        });
    }

    mounts
}

fn resolve_caches(caches: &[CacheConfig]) -> Vec<ResolvedCache> {
    caches
        .iter()
        .map(|cache| ResolvedCache {
            name: cache.name.clone(),
            target: cache.target.clone(),
            source: cache.source.clone(),
            read_only: cache.read_only.unwrap_or(false),
        })
        .collect()
}

fn resolve_secrets(secrets: &[SecretConfig], active_profile: &str) -> Vec<ResolvedSecret> {
    secrets
        .iter()
        .filter(|secret| {
            secret.when_profiles.is_empty()
                || secret
                    .when_profiles
                    .iter()
                    .any(|profile| profile == active_profile)
        })
        .map(|secret| ResolvedSecret {
            name: secret.name.clone(),
            source: secret.source.clone(),
            target: secret.target.clone(),
        })
        .collect()
}

fn resolve_user(config: &Config) -> ResolvedUser {
    match config.identity.as_ref() {
        Some(identity) => match (identity.uid, identity.gid) {
            (Some(uid), Some(gid)) => ResolvedUser::Explicit { uid, gid },
            _ if identity.map_user.unwrap_or(true) => ResolvedUser::KeepId,
            _ => ResolvedUser::Default,
        },
        None => ResolvedUser::KeepId,
    }
}

fn resolve_relative_path(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn join_sandbox_path(mount: &str, relative: &Path) -> String {
    let mut path = mount.trim_end_matches('/').to_string();
    if path.is_empty() {
        path.push('/');
    }

    for component in relative.components() {
        let segment = component.as_os_str().to_string_lossy();
        if segment.is_empty() || segment == "." {
            continue;
        }

        if !path.ends_with('/') {
            path.push('/');
        }
        path.push_str(&segment);
    }

    path
}

fn stable_hash(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn reusable_session_name(config: &Config, workspace_root: &Path, profile_name: &str) -> String {
    if let Some(template) = config
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.container_name.as_ref())
    {
        let workspace_hash = stable_hash(&workspace_root.display().to_string());
        return sanitize_session_name(
            &template
                .replace("{profile}", profile_name)
                .replace("{workspace_hash}", &workspace_hash),
        );
    }

    sanitize_session_name(&format!(
        "sbox-{}-{}",
        stable_hash(&workspace_root.display().to_string()),
        profile_name
    ))
}

fn sanitize_session_name(name: &str) -> String {
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

fn resolve_preset_reference(preset: &str) -> Result<String, SboxError> {
    let reference = match preset {
        "python" => "python:3.13-slim",
        "node" => "node:22-bookworm-slim",
        "rust" => "rust:1-bookworm",
        "go" => "golang:1.24-bookworm",
        "java" => "eclipse-temurin:21-jdk",
        "php" => "php:8.3-cli-bookworm",
        "polyglot" => "ubuntu:24.04",
        _ => {
            return Err(SboxError::UnknownPreset {
                name: preset.to_string(),
            });
        }
    };

    Ok(reference.to_string())
}

fn attach_digest(reference: &str, digest: Option<&str>) -> String {
    match digest {
        Some(digest) if !reference.contains('@') => format!("{reference}@{digest}"),
        _ => reference.to_string(),
    }
}

fn classify_reference_trust(reference: &str, digest: Option<&str>) -> ImageTrust {
    if digest.is_some() || reference.contains("@sha256:") {
        ImageTrust::PinnedDigest
    } else {
        ImageTrust::MutableReference
    }
}

fn detect_package_manager(command: &[String]) -> Option<&'static str> {
    match command.first().map(String::as_str) {
        Some("npm") => Some("npm"),
        Some("pnpm") => Some("pnpm"),
        Some("yarn") => Some("yarn"),
        Some("bun") => Some("bun"),
        Some("uv") => Some("uv"),
        Some("poetry") => Some("poetry"),
        Some("pip") | Some("pip3") => Some("pip"),
        Some("cargo") => Some("cargo"),
        Some("go") => Some("go"),
        Some("composer") => Some("composer"),
        _ => None,
    }
}

fn resolve_lockfile_audit(
    command: &[String],
    project_dir: &Path,
    require_lockfile: Option<bool>,
) -> LockfileAudit {
    let expected_files = expected_lockfiles(command);
    if expected_files.is_empty() {
        return LockfileAudit {
            applicable: false,
            required: require_lockfile.unwrap_or(false),
            present: false,
            expected_files: Vec::new(),
        };
    }

    let present = expected_files
        .iter()
        .any(|candidate| project_dir.join(candidate).exists());

    LockfileAudit {
        applicable: true,
        required: require_lockfile.unwrap_or(true),
        present,
        expected_files: expected_files.into_iter().map(str::to_string).collect(),
    }
}

fn expected_lockfiles(command: &[String]) -> Vec<&'static str> {
    match command.first().map(String::as_str) {
        Some("uv") if command.get(1).map(String::as_str) == Some("sync") => vec!["uv.lock"],
        Some("npm") if npm_lockfile_expected(command) => {
            vec!["package-lock.json", "npm-shrinkwrap.json"]
        }
        Some("pnpm") if pnpm_lockfile_expected(command) => vec!["pnpm-lock.yaml"],
        Some("yarn") if yarn_lockfile_expected(command) => vec!["yarn.lock"],
        Some("bun") if bun_lockfile_expected(command) => vec!["bun.lock", "bun.lockb"],
        Some("poetry") if poetry_lockfile_expected(command) => vec!["poetry.lock"],
        Some("cargo") if cargo_lockfile_expected(command) => vec!["Cargo.lock"],
        Some("composer") if composer_lockfile_expected(command) => vec!["composer.lock"],
        Some("go") if go_lockfile_expected(command) => vec!["go.sum"],
        _ => Vec::new(),
    }
}

fn npm_lockfile_expected(command: &[String]) -> bool {
    if !matches!(
        command.get(1).map(String::as_str),
        Some("install") | Some("ci") | Some("update") | Some("prune") | Some("rebuild")
    )
    {
        return false;
    }

    let has_explicit_target = command.iter().skip(2).any(|arg| {
        !arg.starts_with('-') && (arg.contains('/') || arg.ends_with(".tgz") || arg.contains('@'))
    });

    !has_explicit_target
}

fn pnpm_lockfile_expected(command: &[String]) -> bool {
    matches!(
        command.get(1).map(String::as_str),
        Some("install") | Some("add") | Some("update") | Some("up") | Some("fetch")
    )
}

fn yarn_lockfile_expected(command: &[String]) -> bool {
    matches!(
        command.get(1).map(String::as_str),
        Some("install") | Some("add") | Some("up") | Some("remove")
    )
}

fn bun_lockfile_expected(command: &[String]) -> bool {
    matches!(
        command.get(1).map(String::as_str),
        Some("install") | Some("add") | Some("update")
    )
}

fn poetry_lockfile_expected(command: &[String]) -> bool {
    matches!(
        command.get(1).map(String::as_str),
        Some("install") | Some("sync") | Some("add") | Some("update")
    )
}

fn cargo_lockfile_expected(command: &[String]) -> bool {
    if command.get(1).map(String::as_str) != Some("install") {
        return false;
    }

    command.iter().skip(2).any(|arg| arg == "--path" || arg == "--git")
}

fn composer_lockfile_expected(command: &[String]) -> bool {
    matches!(
        command.get(1).map(String::as_str),
        Some("install") | Some("update") | Some("require")
    )
}

fn go_lockfile_expected(command: &[String]) -> bool {
    matches!(
        command.get(1).map(String::as_str),
        Some("get") | Some("install")
    )
}

fn resolve_script_hook_audit(
    command: &[String],
    environment: &ResolvedEnvironment,
    configured_policy: Option<ScriptPolicy>,
) -> ScriptHookAudit {
    let applicable = is_script_capable_command(command);

    if !applicable {
        return ScriptHookAudit {
            applicable: false,
            blocked: false,
            policy: configured_policy
                .map(script_policy_state)
                .unwrap_or(ScriptPolicyState::Allow),
        };
    }

    let blocked = command.iter().any(|arg| arg == "--ignore-scripts")
        || environment.variables.iter().any(|variable| {
            (variable.name.eq_ignore_ascii_case("npm_config_ignore_scripts")
                || variable
                    .name
                    .eq_ignore_ascii_case("BUN_INSTALL_IGNORE_SCRIPTS"))
                && is_truthy_env_value(&variable.value)
        });
    ScriptHookAudit {
        applicable: true,
        blocked,
        policy: configured_policy
            .map(script_policy_state)
            .unwrap_or(ScriptPolicyState::Allow),
    }
}

fn script_policy_state(policy: ScriptPolicy) -> ScriptPolicyState {
    match policy {
        ScriptPolicy::Allow => ScriptPolicyState::Allow,
        ScriptPolicy::Ignore => ScriptPolicyState::Ignore,
        ScriptPolicy::Block => ScriptPolicyState::Block,
    }
}

fn is_truthy_env_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn is_script_capable_command(command: &[String]) -> bool {
    match (
        detect_package_manager(command),
        command.get(1).map(String::as_str),
    ) {
        (Some("npm"), Some(subcommand)) => {
            matches!(subcommand, "install" | "ci" | "update" | "rebuild")
        }
        (Some("pnpm"), Some(subcommand)) => {
            matches!(subcommand, "install" | "add" | "update" | "up" | "rebuild")
        }
        (Some("yarn"), Some(subcommand)) => matches!(subcommand, "install" | "add" | "up"),
        (Some("bun"), Some(subcommand)) => matches!(subcommand, "install" | "add" | "update"),
        _ => false,
    }
}

fn resolve_audit_hook_audit(command: &[String], hooks: &[AuditHook]) -> AuditHookAudit {
    let configured = hooks
        .iter()
        .map(audit_hook_name)
        .map(str::to_string)
        .collect();
    let runnable = hooks
        .iter()
        .filter_map(|hook| audit_hook_command(hook, command).map(|command| AuditHookExecution {
            name: audit_hook_name(hook).to_string(),
            command,
        }))
        .collect();

    AuditHookAudit {
        configured,
        runnable,
    }
}

fn audit_hook_name(hook: &AuditHook) -> &'static str {
    match hook {
        AuditHook::NpmAudit => "npm-audit",
        AuditHook::PnpmAudit => "pnpm-audit",
        AuditHook::YarnAudit => "yarn-audit",
        AuditHook::PipAudit => "pip-audit",
        AuditHook::CargoAudit => "cargo-audit",
        AuditHook::BunAudit => "bun-audit",
        AuditHook::ComposerAudit => "composer-audit",
        AuditHook::GoAudit => "go-audit",
    }
}

pub(crate) fn audit_hook_command(hook: &AuditHook, command: &[String]) -> Option<Vec<String>> {
    match (hook, detect_package_manager(command)?) {
        (AuditHook::NpmAudit, "npm") => Some(vec![
            "npm".to_string(),
            "audit".to_string(),
            "--audit-level=high".to_string(),
        ]),
        (AuditHook::PnpmAudit, "pnpm") => Some(vec![
            "pnpm".to_string(),
            "audit".to_string(),
            "--audit-level=high".to_string(),
        ]),
        (AuditHook::YarnAudit, "yarn") => Some(vec![
            "yarn".to_string(),
            "npm".to_string(),
            "audit".to_string(),
            "--severity".to_string(),
            "high".to_string(),
        ]),
        (AuditHook::PipAudit, "pip") => Some(vec!["pip-audit".to_string()]),
        (AuditHook::CargoAudit, "cargo") => Some(vec![
            "cargo".to_string(),
            "audit".to_string(),
        ]),
        (AuditHook::BunAudit, "bun") => Some(vec![
            "bun".to_string(),
            "audit".to_string(),
        ]),
        (AuditHook::ComposerAudit, "composer") => Some(vec![
            "composer".to_string(),
            "audit".to_string(),
        ]),
        (AuditHook::GoAudit, "go") => Some(vec![
            "govulncheck".to_string(),
            "./...".to_string(),
        ]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::{
        ImageTrust, ProfileSource, ResolutionTarget, ResolvedImageSource, ResolvedUser,
        ScriptPolicyState, audit_hook_command, resolve_execution_plan,
    };
    use crate::cli::{Cli, Commands, PlanCommand};
    use crate::config::{
        BackendKind,
        load::LoadedConfig,
        model::{
            AuditHook, Config, DispatchRule, EnvironmentConfig, ExecutionMode, ImageConfig,
            ProfileConfig, RuntimeConfig, ScriptPolicy, WorkspaceConfig,
        },
    };
    use std::collections::BTreeMap;

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
            command: Commands::Plan(PlanCommand {
                command: vec!["npm".into(), "install".into()],
            }),
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
                script_policy: None,
                ports: Vec::new(),
                audit_hooks: Vec::new(),
                capabilities: None,
                no_new_privileges: Some(true),
                read_only_rootfs: None,
                reuse_container: None,
                shell: None,
            },
        );
        profiles.insert(
            "install".to_string(),
            ProfileConfig {
                mode: ExecutionMode::Sandbox,
                image: None,
                network: Some("on".to_string()),
                writable: Some(true),
                require_pinned_image: None,
                require_lockfile: None,
                script_policy: None,
                ports: Vec::new(),
                audit_hooks: Vec::new(),
                capabilities: None,
                no_new_privileges: Some(true),
                read_only_rootfs: None,
                reuse_container: None,
                shell: None,
            },
        );

        let mut dispatch = IndexMap::new();
        dispatch.insert(
            "install".to_string(),
            DispatchRule {
                patterns: vec!["npm install".to_string()],
                profile: "install".to_string(),
            },
        );

        Config {
            version: 1,
            runtime: Some(RuntimeConfig {
                backend: Some(BackendKind::Podman),
                rootless: Some(true),
                reuse_container: Some(false),
                container_name: None,
                pull_policy: None,
                strict_security: None,
            }),
            workspace: Some(WorkspaceConfig {
                root: None,
                mount: Some("/workspace".to_string()),
                writable: Some(true),
            }),
            identity: None,
            image: Some(ImageConfig {
                reference: Some("python:3.13-slim".to_string()),
                build: None,
                preset: None,
                digest: None,
                verify_signature: None,
                pull_policy: None,
                tag: None,
            }),
            environment: None,
            mounts: Vec::new(),
            caches: Vec::new(),
            secrets: Vec::new(),
            profiles,
            dispatch,
        }
    }

    fn loaded_config(config: Config) -> LoadedConfig {
        LoadedConfig {
            invocation_dir: PathBuf::from("/workspace/project"),
            workspace_root: PathBuf::from("/workspace/project"),
            config_path: PathBuf::from("/workspace/project/sbox.yaml"),
            config,
        }
    }

    use std::path::PathBuf;

    #[test]
    fn selects_dispatch_profile_in_declaration_order() {
        let cli = base_cli();
        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(base_config()),
            ResolutionTarget::Plan,
            &["npm".into(), "install".into()],
        )
        .expect("resolution should succeed");

        assert_eq!(plan.profile_name, "install");
        assert!(matches!(
            plan.image.source,
            ResolvedImageSource::Reference(ref image) if image == "python:3.13-slim"
        ));
        assert_eq!(plan.image.trust, ImageTrust::MutableReference);
        assert!(matches!(plan.user, ResolvedUser::KeepId));
        match plan.profile_source {
            ProfileSource::Dispatch { rule_name, pattern } => {
                assert_eq!(rule_name, "install");
                assert_eq!(pattern, "npm install");
            }
            other => panic!("expected dispatch source, got {other:?}"),
        }
    }

    #[test]
    fn falls_back_to_default_profile_when_no_dispatch_matches() {
        let cli = base_cli();
        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(base_config()),
            ResolutionTarget::Plan,
            &["echo".into(), "hello".into()],
        )
        .expect("resolution should succeed");

        assert_eq!(plan.profile_name, "default");
        assert!(matches!(plan.profile_source, ProfileSource::DefaultProfile));
        assert_eq!(plan.policy.cap_drop, Vec::<String>::new());
    }

    #[test]
    fn workspace_mount_becomes_read_only_when_profile_is_not_writable() {
        let cli = base_cli();
        let mut config = base_config();
        config
            .profiles
            .get_mut("default")
            .expect("default profile exists")
            .writable = Some(false);

        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(config),
            ResolutionTarget::Plan,
            &["echo".into(), "hello".into()],
        )
        .expect("resolution should succeed");

        let workspace_mount = plan
            .mounts
            .iter()
            .find(|mount| mount.is_workspace)
            .expect("workspace mount should be present");

        assert!(workspace_mount.read_only);
        assert!(!plan.policy.writable);
    }

    #[test]
    fn runtime_reuse_container_enables_reusable_session_name() {
        let cli = base_cli();
        let mut config = base_config();
        config
            .runtime
            .as_mut()
            .expect("runtime exists")
            .reuse_container = Some(true);

        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(config),
            ResolutionTarget::Plan,
            &["echo".into(), "hello".into()],
        )
        .expect("resolution should succeed");

        assert!(plan.policy.reuse_container);
        assert!(
            plan.policy
                .reusable_session_name
                .as_deref()
                .is_some_and(|name| name.starts_with("sbox-"))
        );
    }

    #[test]
    fn audit_marks_install_style_profiles() {
        let cli = base_cli();
        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(base_config()),
            ResolutionTarget::Plan,
            &["npm".into(), "install".into()],
        )
        .expect("resolution should succeed");

        assert!(plan.audit.install_style);
        assert!(!plan.audit.trusted_image_required);
    }

    #[test]
    fn resolves_known_presets_to_references() {
        let cli = base_cli();
        let mut config = base_config();
        config.image = Some(ImageConfig {
            reference: None,
            build: None,
            preset: Some("python".to_string()),
            digest: None,
            verify_signature: None,
            pull_policy: None,
            tag: None,
        });

        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(config),
            ResolutionTarget::Plan,
            &["python".into(), "--version".into()],
        )
        .expect("resolution should succeed");

        assert!(matches!(
            plan.image.source,
            ResolvedImageSource::Reference(ref image) if image == "python:3.13-slim"
        ));
    }

    #[test]
    fn profile_can_require_trusted_image() {
        let cli = base_cli();
        let mut config = base_config();
        config
            .profiles
            .get_mut("install")
            .expect("install profile exists")
            .require_pinned_image = Some(true);

        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(config),
            ResolutionTarget::Plan,
            &["npm".into(), "install".into()],
        )
        .expect("resolution should succeed");

        assert!(plan.audit.install_style);
        assert!(plan.audit.trusted_image_required);
    }

    #[test]
    fn image_digest_pins_reference_trust() {
        let cli = base_cli();
        let mut config = base_config();
        config.image = Some(ImageConfig {
            reference: Some("python:3.13-slim".to_string()),
            build: None,
            preset: None,
            digest: Some("sha256:deadbeef".to_string()),
            verify_signature: Some(true),
            pull_policy: None,
            tag: None,
        });

        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(config),
            ResolutionTarget::Plan,
            &["python".into(), "--version".into()],
        )
        .expect("resolution should succeed");

        assert!(matches!(
            plan.image.source,
            ResolvedImageSource::Reference(ref image)
                if image == "python:3.13-slim@sha256:deadbeef"
        ));
        assert_eq!(plan.image.trust, ImageTrust::PinnedDigest);
        assert!(plan.image.verify_signature);
    }

    #[test]
    fn profile_can_require_lockfile_without_strict_mode() {
        let cli = base_cli();
        let mut config = base_config();
        config
            .profiles
            .get_mut("install")
            .expect("install profile exists")
            .require_lockfile = Some(true);

        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(config),
            ResolutionTarget::Plan,
            &["npm".into(), "install".into()],
        )
        .expect("resolution should succeed");

        assert!(plan.audit.lockfile.applicable);
        assert!(plan.audit.lockfile.required);
    }

    #[test]
    fn script_policy_detects_ignore_scripts_from_environment() {
        let cli = base_cli();
        let mut config = base_config();
        config.environment = Some(EnvironmentConfig {
            pass_through: Vec::new(),
            set: BTreeMap::from([(
                "npm_config_ignore_scripts".to_string(),
                "true".to_string(),
            )]),
            deny: Vec::new(),
        });
        config
            .profiles
            .get_mut("install")
            .expect("install profile exists")
            .script_policy = Some(ScriptPolicy::Ignore);

        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(config),
            ResolutionTarget::Plan,
            &["npm".into(), "install".into()],
        )
        .expect("resolution should succeed");

        assert!(plan.audit.script_hooks.applicable);
        assert!(plan.audit.script_hooks.blocked);
        assert_eq!(plan.audit.script_hooks.policy, ScriptPolicyState::Ignore);
    }

    #[test]
    fn audit_hook_command_matches_package_manager() {
        let npm = audit_hook_command(
            &AuditHook::NpmAudit,
            &["npm".into(), "install".into()],
        )
        .expect("npm hook should apply");
        assert_eq!(npm, vec!["npm", "audit", "--audit-level=high"]);

        let pip = audit_hook_command(
            &AuditHook::PipAudit,
            &["pip".into(), "install".into(), "requests".into()],
        )
        .expect("pip hook should apply");
        assert_eq!(pip, vec!["pip-audit"]);

        let cargo = audit_hook_command(
            &AuditHook::CargoAudit,
            &["cargo".into(), "install".into(), "--path".into(), ".".into()],
        )
        .expect("cargo hook should apply");
        assert_eq!(cargo, vec!["cargo", "audit"]);

        let bun = audit_hook_command(
            &AuditHook::BunAudit,
            &["bun".into(), "install".into()],
        )
        .expect("bun hook should apply");
        assert_eq!(bun, vec!["bun", "audit"]);

        let composer = audit_hook_command(
            &AuditHook::ComposerAudit,
            &["composer".into(), "install".into()],
        )
        .expect("composer hook should apply");
        assert_eq!(composer, vec!["composer", "audit"]);

        let go = audit_hook_command(
            &AuditHook::GoAudit,
            &["go".into(), "get".into(), "./...".into()],
        )
        .expect("go hook should apply");
        assert_eq!(go, vec!["govulncheck", "./..."]);

        assert!(audit_hook_command(
            &AuditHook::NpmAudit,
            &["uv".into(), "sync".into()],
        )
        .is_none());
    }

    #[test]
    fn bun_install_has_lockfile_audit() {
        let cli = base_cli();
        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(base_config()),
            ResolutionTarget::Plan,
            &["bun".into(), "install".into()],
        )
        .expect("resolution should succeed");

        assert_eq!(plan.audit.package_manager.as_deref(), Some("bun"));
        assert!(plan.audit.lockfile.applicable);
        assert_eq!(
            plan.audit.lockfile.expected_files,
            vec!["bun.lock".to_string(), "bun.lockb".to_string()]
        );
        assert!(plan.audit.script_hooks.applicable);
    }

    #[test]
    fn poetry_install_has_lockfile_audit() {
        let cli = base_cli();
        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(base_config()),
            ResolutionTarget::Plan,
            &["poetry".into(), "install".into()],
        )
        .expect("resolution should succeed");

        assert_eq!(plan.audit.package_manager.as_deref(), Some("poetry"));
        assert!(plan.audit.lockfile.applicable);
        assert_eq!(plan.audit.lockfile.expected_files, vec!["poetry.lock".to_string()]);
        assert!(!plan.audit.script_hooks.applicable);
    }

    #[test]
    fn cargo_install_with_path_has_lockfile_audit() {
        let cli = base_cli();
        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(base_config()),
            ResolutionTarget::Plan,
            &["cargo".into(), "install".into(), "--path".into(), ".".into()],
        )
        .expect("resolution should succeed");

        assert_eq!(plan.audit.package_manager.as_deref(), Some("cargo"));
        assert!(plan.audit.lockfile.applicable);
        assert_eq!(plan.audit.lockfile.expected_files, vec!["Cargo.lock".to_string()]);
    }
}
