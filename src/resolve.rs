use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::cli::{Cli, CliBackendKind, CliExecutionMode};
use crate::config::{
    BackendKind, ImageConfig, LoadedConfig,
    model::{
        CacheConfig, Config, EnvironmentConfig, ExecutionMode, MountType, ProfileConfig,
        ProfileRole, SecretConfig,
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
    pub lockfile: LockfileAudit,
    /// Pre-run commands to execute on the host before the sandboxed command.
    /// Each inner Vec is a tokenised command (argv), parsed from the profile's `pre_run` strings.
    pub pre_run: Vec<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct LockfileAudit {
    pub applicable: bool,
    pub required: bool,
    pub present: bool,
    pub expected_files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedImage {
    pub description: String,
    pub source: ResolvedImageSource,
    pub trust: ImageTrust,
    pub verify_signature: bool,
}

#[derive(Debug, Clone)]
pub enum ResolvedImageSource {
    Reference(String),
    Build { recipe_path: PathBuf, tag: String },
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
    pub pull_policy: Option<String>,
    /// Resolved allow-list: `(hostname, ip)` pairs injected as `--add-host` entries.
    /// Empty means no restriction (full network or network off).
    pub network_allow: Vec<(String, String)>,
    /// Original glob/regex patterns from `network_allow` that were expanded at resolve time.
    /// Stored for plan display. Enforcement is via the resolved base-domain IPs in `network_allow`.
    pub network_allow_patterns: Vec<String>,
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
    /// When true, the host source directory is created automatically if it does not exist.
    pub create: bool,
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
        profile,
        &resolved_workspace.root,
        &resolved_workspace.mount,
        policy.writable,
    );
    let caches = resolve_caches(&config.caches);
    let secrets = resolve_secrets(
        &config.secrets,
        &profile_resolution.name,
        profile.role.as_ref(),
    );
    let rootless = config
        .runtime
        .as_ref()
        .and_then(|rt| rt.rootless)
        .unwrap_or(true);
    let user = resolve_user(config, rootless);
    let install_style = is_install_style(&profile.role, &profile_resolution.name);
    let audit = ExecutionAudit {
        install_style,
        trusted_image_required: profile.require_pinned_image.unwrap_or(false)
            || config
                .runtime
                .as_ref()
                .and_then(|rt| rt.require_pinned_image)
                .unwrap_or(false),
        sensitive_pass_through_vars: resolved_sensitive_pass_through_vars(&environment),
        lockfile: resolve_lockfile_audit(
            &profile.lockfile_files,
            install_style,
            &resolved_workspace.effective_host_dir,
            profile.require_lockfile,
        ),
        pre_run: parse_pre_run_commands(&profile.pre_run),
    };

    Ok(ExecutionPlan {
        command_string: dispatch::command_string(command),
        command: command.to_vec(),
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
            .unwrap_or_else(detect_backend),
    }
}

fn detect_backend() -> BackendKind {
    // Probe PATH: prefer podman (rootless-first), fall back to docker.
    if which_on_path("podman") {
        return BackendKind::Podman;
    }
    if which_on_path("docker") {
        return BackendKind::Docker;
    }
    // Default to podman — execution will fail with a clear "backend unavailable" error.
    BackendKind::Podman
}

pub(crate) fn which_on_path(name: &str) -> bool {
    let Some(path_os) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path_os) {
        #[cfg(windows)]
        {
            // On Windows, executables always have an extension — never treat an
            // extensionless file as runnable (avoids false-positive backend detection).
            for ext in &[".exe", ".cmd", ".bat"] {
                let candidate = dir.join(format!("{name}{ext}"));
                if candidate.is_file() {
                    return true;
                }
            }
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let candidate = dir.join(name);
            if candidate.is_file()
                && candidate
                    .metadata()
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
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

    Err(SboxError::ConfigValidation {
        message: "`image` must define exactly one of `ref`, `build`, or `preset`".to_string(),
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

    let pull_policy = profile
        .image
        .as_ref()
        .and_then(|img| img.pull_policy.as_ref())
        .or_else(|| {
            config
                .image
                .as_ref()
                .and_then(|img| img.pull_policy.as_ref())
        })
        .or_else(|| {
            config
                .runtime
                .as_ref()
                .and_then(|rt| rt.pull_policy.as_ref())
        })
        .map(pull_policy_flag);

    let network_allow_resolved = resolve_network_allow(&profile.network_allow, &profile.network);

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
        pull_policy,
        network_allow: network_allow_resolved.0,
        network_allow_patterns: network_allow_resolved.1,
    }
}

fn pull_policy_flag(policy: &crate::config::model::PullPolicy) -> String {
    match policy {
        crate::config::model::PullPolicy::Always => "always".to_string(),
        crate::config::model::PullPolicy::IfMissing => "missing".to_string(),
        crate::config::model::PullPolicy::Never => "never".to_string(),
    }
}

fn resolve_capabilities(profile: &ProfileConfig) -> (Vec<String>, Vec<String>) {
    match &profile.capabilities {
        Some(crate::config::model::CapabilitiesSpec::Structured(cfg)) => {
            (cfg.drop.clone(), cfg.add.clone())
        }
        Some(crate::config::model::CapabilitiesSpec::Keyword(keyword)) if keyword == "drop-all" => {
            (vec!["all".to_string()], Vec::new())
        }
        Some(crate::config::model::CapabilitiesSpec::List(values)) => (Vec::new(), values.clone()),
        Some(crate::config::model::CapabilitiesSpec::Keyword(_)) => {
            // Rejected by validation; unreachable in a valid config.
            (Vec::new(), Vec::new())
        }
        None => (Vec::new(), Vec::new()),
    }
}

/// Resolve `network_allow` entries to `(hostname, ip)` pairs plus raw patterns.
///
/// Each entry is either:
/// - An exact hostname (`registry.npmjs.org`) → DNS-resolved to `(hostname, ip)` pairs
/// - A glob pattern (`*.npmjs.org`) → base domain resolved + original pattern recorded
/// - A regex-style prefix pattern (`.*\.npmjs\.org`) → same as glob
///
/// Returns `(resolved_pairs, patterns)`.
/// Only active when `network` is not `off`.
fn resolve_network_allow(
    domains: &[String],
    network: &Option<String>,
) -> (Vec<(String, String)>, Vec<String>) {
    if domains.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if network.as_deref() == Some("off") {
        return (Vec::new(), Vec::new());
    }

    let mut entries: Vec<(String, String)> = Vec::new();
    let mut patterns: Vec<String> = Vec::new();

    for entry in domains {
        if let Some(base) = extract_pattern_base(entry) {
            // It's a glob/regex pattern — store the original, expand to known subdomains.
            patterns.push(entry.clone());
            for hostname in expand_pattern_hosts(&base) {
                resolve_hostname_into(&hostname, &mut entries);
            }
        } else {
            resolve_hostname_into(entry, &mut entries);
        }
    }

    (entries, patterns)
}

/// Expand a base domain to the set of hostnames to resolve for a wildcard pattern.
///
/// For well-known package registry domains we enumerate the subdomains that package
/// managers actually use, so `*.npmjs.org` resolves `registry.npmjs.org` and friends
/// rather than just `npmjs.org` itself.  For unknown domains the base is returned as-is.
fn expand_pattern_hosts(base: &str) -> Vec<String> {
    const KNOWN: &[(&str, &[&str])] = &[
        // npm / yarn
        (
            "npmjs.org",
            &["registry.npmjs.org", "npmjs.org", "www.npmjs.org"],
        ),
        ("yarnpkg.com", &["registry.yarnpkg.com", "yarnpkg.com"]),
        // Python
        ("pypi.org", &["pypi.org", "files.pythonhosted.org"]),
        (
            "pythonhosted.org",
            &["files.pythonhosted.org", "pythonhosted.org"],
        ),
        // Rust
        (
            "crates.io",
            &["crates.io", "static.crates.io", "index.crates.io"],
        ),
        // Go
        (
            "golang.org",
            &["proxy.golang.org", "sum.golang.org", "golang.org"],
        ),
        ("go.dev", &["proxy.golang.dev", "sum.golang.dev", "go.dev"]),
        // Ruby
        (
            "rubygems.org",
            &["rubygems.org", "api.rubygems.org", "index.rubygems.org"],
        ),
        // Java / Gradle / Maven
        ("maven.org", &["repo1.maven.org", "central.maven.org"]),
        (
            "gradle.org",
            &["plugins.gradle.org", "services.gradle.org", "gradle.org"],
        ),
        // GitHub (source deps)
        (
            "github.com",
            &[
                "github.com",
                "api.github.com",
                "raw.githubusercontent.com",
                "objects.githubusercontent.com",
                "codeload.github.com",
            ],
        ),
        (
            "githubusercontent.com",
            &[
                "raw.githubusercontent.com",
                "objects.githubusercontent.com",
                "avatars.githubusercontent.com",
            ],
        ),
        // Docker / OCI registries
        (
            "docker.io",
            &[
                "registry-1.docker.io",
                "auth.docker.io",
                "production.cloudflare.docker.com",
            ],
        ),
        ("ghcr.io", &["ghcr.io"]),
        ("gcr.io", &["gcr.io"]),
    ];

    for (domain, subdomains) in KNOWN {
        if base == *domain {
            return subdomains.iter().map(|s| s.to_string()).collect();
        }
    }

    // Unknown domain — resolve just the base; a DNS proxy would be needed for full wildcard coverage.
    vec![base.to_string()]
}

/// Return the concrete base domain for a glob/regex pattern, or `None` if the entry is exact.
///
/// Supported forms:
/// - `*.example.com`        → `example.com`
/// - `.*\.example\.com`     → `example.com`   (regex prefix `.*\.`)
/// - `.example.com`         → `example.com`   (leading-dot notation)
fn extract_pattern_base(entry: &str) -> Option<String> {
    // Glob: *.example.com
    if let Some(rest) = entry.strip_prefix("*.") {
        return Some(rest.to_string());
    }
    // Regex: .*\.example\.com — strip leading `.*\.` and unescape `\.` → `.`
    if let Some(rest) = entry.strip_prefix(".*\\.") {
        return Some(rest.replace("\\.", "."));
    }
    // Leading-dot notation: .example.com
    if let Some(rest) = entry.strip_prefix('.')
        && !rest.is_empty()
    {
        return Some(rest.to_string());
    }
    None
}

/// DNS-resolve `hostname` and append unique `(hostname, ip)` pairs into `entries`.
fn resolve_hostname_into(hostname: &str, entries: &mut Vec<(String, String)>) {
    let addr = format!("{hostname}:443");
    if let Ok(addrs) = std::net::ToSocketAddrs::to_socket_addrs(&addr.as_str()) {
        for socket_addr in addrs {
            let ip = socket_addr.ip().to_string();
            if !entries.iter().any(|(h, a)| h == hostname && a == &ip) {
                entries.push((hostname.to_string(), ip));
            }
        }
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
        if denied.contains(name.as_str()) {
            continue;
        }
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

/// Returns true when the resolved profile declares itself as an install profile.
/// Falls back to well-known profile-name conventions when no explicit role is set.
fn is_install_style(role: &Option<ProfileRole>, profile_name: &str) -> bool {
    match role {
        Some(ProfileRole::Install) => true,
        Some(_) => false,
        None => {
            matches!(
                profile_name,
                "install" | "deps" | "dependency-install" | "bootstrap"
            ) || profile_name.contains("install")
        }
    }
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

/// Walk `dir` recursively and collect paths of files that match `pattern`.
/// Skips large build/tool directories (.git, node_modules, target, .venv) for performance.
/// Does not follow symlinks.
fn collect_excluded_files(
    workspace_root: &Path,
    dir: &Path,
    pattern: &str,
    out: &mut Vec<PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        // Never follow symlinks — avoids loops and unintended host path access
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip large build/tool dirs that don't contain user credentials
            if matches!(
                name,
                ".git"
                    | "node_modules"
                    | "target"
                    | ".venv"
                    | "__pycache__"
                    | "vendor"
                    | "dist"
                    | "build"
                    | ".cache"
                    | ".gradle"
                    | ".tox"
            ) {
                continue;
            }
            collect_excluded_files(workspace_root, &path, pattern, out);
        } else if file_type.is_file()
            && let Ok(rel) = path.strip_prefix(workspace_root)
        {
            let rel_str = rel.to_string_lossy();
            if exclude_pattern_matches(&rel_str, pattern) {
                out.push(path);
            }
        }
    }
}

/// Return true if `relative_path` (e.g. `"config/.env"`) matches `pattern`.
///
/// Pattern rules:
/// - Leading `**/` is stripped; the rest is matched against the filename only
///   unless it contains a `/`, in which case it is matched against the full relative path.
/// - `*` matches any sequence of characters within a single path component.
pub(crate) fn exclude_pattern_matches(relative_path: &str, pattern: &str) -> bool {
    let effective = pattern.trim_start_matches("**/");
    if effective.contains('/') {
        // Path-relative pattern: match against the full relative path
        glob_match(relative_path, effective)
    } else {
        // Filename-only pattern: match only the last path component
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        glob_match(filename, effective)
    }
}

/// Simple glob matcher supporting `*` wildcards (no path-separator crossing).
pub(crate) fn glob_match(s: &str, pattern: &str) -> bool {
    if !pattern.contains('*') {
        return s == pattern;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut remaining = s;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !remaining.starts_with(part) {
                return false;
            }
            remaining = &remaining[part.len()..];
        } else if i == parts.len() - 1 {
            return remaining.ends_with(part);
        } else {
            match remaining.find(part) {
                Some(pos) => remaining = &remaining[pos + part.len()..],
                None => return false,
            }
        }
    }
    true
}

fn resolve_mounts(
    config: &Config,
    profile: &ProfileConfig,
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
        create: false,
    }];

    // When the workspace is read-only, inject rw bind mounts for each writable_path.
    // Profile-level writable_paths override workspace-level when set.
    // These are mounted after the workspace so Podman's mount ordering gives them precedence.
    if !workspace_writable {
        let writable_paths: &[String] = profile.writable_paths.as_deref().unwrap_or_else(|| {
            config
                .workspace
                .as_ref()
                .map(|ws| ws.writable_paths.as_slice())
                .unwrap_or(&[])
        });
        for rel_path in writable_paths {
            mounts.push(ResolvedMount {
                kind: "bind".to_string(),
                source: Some(workspace_root.join(rel_path)),
                target: format!("{workspace_mount}/{rel_path}"),
                read_only: false,
                is_workspace: true,
                create: true,
            });
        }
    }

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
            create: false,
        });
    }

    // Mask credential/secret files with /dev/null overlays — independent of workspace writability.
    // These mounts are added last so they take precedence over the workspace bind mount.
    let exclude_patterns = config
        .workspace
        .as_ref()
        .map(|ws| ws.exclude_paths.as_slice())
        .unwrap_or(&[]);
    for pattern in exclude_patterns {
        let mut matched = Vec::new();
        collect_excluded_files(workspace_root, workspace_root, pattern, &mut matched);
        for host_path in matched {
            if let Ok(rel) = host_path.strip_prefix(workspace_root) {
                let target = format!("{workspace_mount}/{}", rel.display());
                mounts.push(ResolvedMount {
                    kind: "mask".to_string(),
                    source: None,
                    target,
                    read_only: true,
                    is_workspace: true,
                    create: false,
                });
            }
        }
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

fn resolve_secrets(
    secrets: &[SecretConfig],
    active_profile: &str,
    active_role: Option<&ProfileRole>,
) -> Vec<ResolvedSecret> {
    secrets
        .iter()
        .filter(|secret| {
            // when_profiles: include only if empty or profile name matches
            let profile_ok = secret.when_profiles.is_empty()
                || secret.when_profiles.iter().any(|p| p == active_profile);

            // deny_roles: exclude if the active profile's role is in the deny list
            let role_ok = active_role
                .map(|role| !secret.deny_roles.contains(role))
                .unwrap_or(true);

            profile_ok && role_ok
        })
        .map(|secret| ResolvedSecret {
            name: secret.name.clone(),
            source: secret.source.clone(),
            target: secret.target.clone(),
        })
        .collect()
}

fn resolve_user(config: &Config, rootless: bool) -> ResolvedUser {
    match config.identity.as_ref() {
        Some(identity) => match (identity.uid, identity.gid) {
            (Some(uid), Some(gid)) => ResolvedUser::Explicit { uid, gid },
            _ if identity.map_user.unwrap_or(rootless) => ResolvedUser::KeepId,
            _ => ResolvedUser::Default,
        },
        None if rootless => ResolvedUser::KeepId,
        None => ResolvedUser::Default,
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

/// Resolve lockfile audit from the profile's explicit `lockfile_files` list.
fn resolve_lockfile_audit(
    lockfile_files: &[String],
    install_style: bool,
    project_dir: &Path,
    require_lockfile: Option<bool>,
) -> LockfileAudit {
    if !install_style || lockfile_files.is_empty() {
        return LockfileAudit {
            applicable: false,
            required: require_lockfile.unwrap_or(false),
            present: false,
            expected_files: Vec::new(),
        };
    }

    let present = lockfile_files
        .iter()
        .any(|candidate| project_dir.join(candidate).exists());

    LockfileAudit {
        applicable: true,
        required: require_lockfile.unwrap_or(true),
        present,
        expected_files: lockfile_files.to_vec(),
    }
}

/// Parse `pre_run` strings (e.g. `"npm audit --audit-level=high"`) into argv vecs.
/// Uses simple whitespace splitting — no shell quoting support needed for command names.
fn parse_pre_run_commands(pre_run: &[String]) -> Vec<Vec<String>> {
    pre_run
        .iter()
        .filter_map(|s| {
            let tokens: Vec<String> = s.split_whitespace().map(str::to_string).collect();
            if tokens.is_empty() {
                None
            } else {
                Some(tokens)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::{
        ImageTrust, ProfileSource, ResolutionTarget, ResolvedImageSource, ResolvedUser,
        resolve_execution_plan,
    };
    use crate::cli::{Cli, Commands, PlanCommand};
    use crate::config::{
        BackendKind,
        load::LoadedConfig,
        model::{
            Config, DispatchRule, ExecutionMode, ImageConfig, ProfileConfig, ProfileRole,
            RuntimeConfig, WorkspaceConfig,
        },
    };

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
                show_command: false,
                audit: false,
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
                role: None,
                lockfile_files: Vec::new(),
                pre_run: Vec::new(),
                network_allow: Vec::new(),
                ports: Vec::new(),
                capabilities: None,
                no_new_privileges: Some(true),
                read_only_rootfs: None,
                reuse_container: None,
                shell: None,

                writable_paths: None,
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
                role: Some(ProfileRole::Install),
                lockfile_files: Vec::new(),
                pre_run: Vec::new(),
                network_allow: Vec::new(),
                ports: Vec::new(),
                capabilities: None,
                no_new_privileges: Some(true),
                read_only_rootfs: None,
                reuse_container: None,
                shell: None,

                writable_paths: None,
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
                require_pinned_image: None,
            }),
            workspace: Some(WorkspaceConfig {
                root: None,
                mount: Some("/workspace".to_string()),
                writable: Some(true),
                writable_paths: Vec::new(),
                exclude_paths: Vec::new(),
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

            package_manager: None,
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
    fn install_role_marks_install_style() {
        let cli = base_cli();
        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(base_config()),
            ResolutionTarget::Plan,
            &["npm".into(), "install".into()],
        )
        .expect("resolution should succeed");

        // dispatched to "install" profile which has role: install
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
    fn profile_lockfile_files_drive_lockfile_audit() {
        let cli = base_cli();
        let mut config = base_config();
        let profile = config
            .profiles
            .get_mut("install")
            .expect("install profile exists");
        profile.require_lockfile = Some(true);
        profile.lockfile_files = vec![
            "package-lock.json".to_string(),
            "npm-shrinkwrap.json".to_string(),
        ];

        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(config),
            ResolutionTarget::Plan,
            &["npm".into(), "install".into()],
        )
        .expect("resolution should succeed");

        assert!(plan.audit.lockfile.applicable);
        assert!(plan.audit.lockfile.required);
        assert_eq!(
            plan.audit.lockfile.expected_files,
            vec!["package-lock.json", "npm-shrinkwrap.json"]
        );
    }

    #[test]
    fn pre_run_parses_into_argv_vecs() {
        let cli = base_cli();
        let mut config = base_config();
        config
            .profiles
            .get_mut("install")
            .expect("install profile exists")
            .pre_run = vec![
            "npm audit --audit-level=high".to_string(),
            "echo done".to_string(),
        ];

        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(config),
            ResolutionTarget::Plan,
            &["npm".into(), "install".into()],
        )
        .expect("resolution should succeed");

        assert_eq!(plan.audit.pre_run.len(), 2);
        assert_eq!(
            plan.audit.pre_run[0],
            vec!["npm", "audit", "--audit-level=high"]
        );
        assert_eq!(plan.audit.pre_run[1], vec!["echo", "done"]);
    }

    #[test]
    fn no_role_profile_name_heuristic_still_marks_install_style() {
        let cli = base_cli();
        let mut config = base_config();
        // "deps" profile name matches the heuristic even without an explicit role
        config.profiles.insert(
            "deps".to_string(),
            ProfileConfig {
                mode: ExecutionMode::Sandbox,
                image: None,
                network: Some("on".to_string()),
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
                shell: None,

                writable_paths: None,
            },
        );
        config.dispatch.insert(
            "uv-sync".to_string(),
            crate::config::model::DispatchRule {
                patterns: vec!["uv sync".to_string()],
                profile: "deps".to_string(),
            },
        );

        let plan = resolve_execution_plan(
            &cli,
            &loaded_config(config),
            ResolutionTarget::Plan,
            &["uv".into(), "sync".into()],
        )
        .expect("resolution should succeed");

        assert_eq!(plan.profile_name, "deps");
        assert!(plan.audit.install_style); // heuristic via name "deps"
    }

    #[test]
    fn glob_match_exact_and_prefix_suffix_wildcards() {
        use super::glob_match;

        // Exact match (no wildcard)
        assert!(glob_match("file.pem", "file.pem"));
        assert!(!glob_match("file.pem", "other.pem"));

        // Suffix wildcard: *.pem
        assert!(glob_match("file.pem", "*.pem"));
        assert!(glob_match(".pem", "*.pem"));
        assert!(!glob_match("pem", "*.pem"));
        assert!(!glob_match("file.pem.bak", "*.pem"));

        // Prefix wildcard: .env.*
        assert!(glob_match(".env.local", ".env.*"));
        assert!(glob_match(".env.production", ".env.*"));
        assert!(!glob_match(".env", ".env.*"));

        // Trailing wildcard: secrets*
        assert!(glob_match("secrets", "secrets*"));
        assert!(glob_match("secrets.json", "secrets*"));
        assert!(!glob_match("not-secrets", "secrets*"));

        // Both-ends wildcard: *.key.*
        assert!(glob_match("my.key.bak", "*.key.*"));
        assert!(glob_match("a.key.b", "*.key.*"));
        assert!(!glob_match("key.bak", "*.key.*"));
        assert!(!glob_match("my.key", "*.key.*"));

        // Three-wildcard pattern: a*b*c
        assert!(glob_match("abc", "a*b*c"));
        assert!(glob_match("aXbYc", "a*b*c"));
        assert!(glob_match("abbc", "a*b*c"));
        assert!(!glob_match("ac", "a*b*c"));
        assert!(!glob_match("aXbYd", "a*b*c"));

        // Pattern with no anchoring (*a*b*)
        assert!(glob_match("XaYbZ", "*a*b*"));
        assert!(glob_match("ab", "*a*b*"));
        assert!(!glob_match("ba", "*a*b*"));
        assert!(!glob_match("XaZ", "*a*b*"));
    }

    #[test]
    fn glob_match_path_patterns() {
        use super::exclude_pattern_matches;

        // Filename-only pattern (no slash): matches against filename
        assert!(exclude_pattern_matches("dir/file.pem", "*.pem"));
        assert!(exclude_pattern_matches("deep/nested/file.key", "*.key"));
        assert!(!exclude_pattern_matches("dir/file.pem.bak", "*.pem"));

        // Leading **/ is stripped before matching
        assert!(exclude_pattern_matches("dir/file.pem", "**/*.pem"));
        assert!(exclude_pattern_matches("a/b/c.key", "**/*.key"));

        // Path-relative pattern (contains slash): matches full relative path
        assert!(exclude_pattern_matches("secrets/prod.json", "secrets/*"));
        assert!(!exclude_pattern_matches("other/prod.json", "secrets/*"));
        assert!(exclude_pattern_matches(
            "config/.env.local",
            "config/.env.*"
        ));
    }
}
