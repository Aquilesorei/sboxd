use std::collections::BTreeMap;
use std::path::PathBuf;

use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub version: u32,

    #[serde(default)]
    pub runtime: Option<RuntimeConfig>,

    #[serde(default)]
    pub workspace: Option<WorkspaceConfig>,

    #[serde(default)]
    pub identity: Option<IdentityConfig>,

    #[serde(default)]
    pub image: Option<ImageConfig>,

    #[serde(default)]
    pub environment: Option<EnvironmentConfig>,

    #[serde(default)]
    pub mounts: Vec<MountConfig>,

    #[serde(default)]
    pub caches: Vec<CacheConfig>,

    #[serde(default)]
    pub secrets: Vec<SecretConfig>,

    #[serde(default)]
    pub profiles: IndexMap<String, ProfileConfig>,

    #[serde(default)]
    pub dispatch: IndexMap<String, DispatchRule>,

    #[serde(default)]
    pub package_manager: Option<PackageManagerConfig>,
}

/// Top-level `package_manager:` block. Generates install/build/default profiles and dispatch
/// rules automatically. Zero additional config required — just specify the package manager name.
#[derive(Debug, Clone, Deserialize)]
pub struct PackageManagerConfig {
    /// One of: npm, yarn, pnpm, bun, uv, pip, poetry, cargo, go
    pub name: String,

    /// Override the preset's install writable paths.
    #[serde(default)]
    pub install_writable: Option<Vec<String>>,

    /// Override the preset's build writable paths.
    #[serde(default)]
    pub build_writable: Option<Vec<String>>,

    /// Override the preset's network_allow list for the install profile.
    #[serde(default)]
    pub network_allow: Option<Vec<String>>,

    /// Commands to run on the host before the sandboxed install. If any fails, execution is refused.
    #[serde(default)]
    pub pre_run: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    Podman,
    Docker,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PullPolicy {
    IfMissing,
    Always,
    Never,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeConfig {
    pub backend: Option<BackendKind>,
    pub rootless: Option<bool>,
    pub strict_security: Option<bool>,
    pub reuse_container: Option<bool>,
    pub container_name: Option<String>,
    pub pull_policy: Option<PullPolicy>,
    pub require_pinned_image: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceConfig {
    pub root: Option<PathBuf>,
    pub mount: Option<String>,
    pub writable: Option<bool>,
    #[serde(default)]
    pub writable_paths: Vec<String>,
    /// Files/patterns to mask inside the container with /dev/null, preventing credential theft.
    /// Supports glob wildcards: `*.pem`, `.env.*`, `**/*.key`. Matched relative to workspace root.
    #[serde(default)]
    pub exclude_paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IdentityConfig {
    pub map_user: Option<bool>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageConfig {
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub build: Option<PathBuf>,
    pub preset: Option<String>,
    pub digest: Option<String>,
    pub verify_signature: Option<bool>,
    pub pull_policy: Option<PullPolicy>,
    pub tag: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct EnvironmentConfig {
    #[serde(default)]
    pub pass_through: Vec<String>,

    #[serde(default)]
    pub set: BTreeMap<String, String>,

    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MountType {
    Bind,
    Tmpfs,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MountConfig {
    pub source: Option<PathBuf>,
    pub target: Option<String>,
    #[serde(rename = "type")]
    pub mount_type: MountType,
    pub read_only: Option<bool>,
    pub create: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    pub name: String,
    pub target: String,
    pub source: Option<String>,
    pub read_only: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecretConfig {
    pub name: String,
    pub source: String,
    pub target: String,

    /// Include this secret only when the active profile matches one of these names.
    /// Empty means "include in all profiles".
    #[serde(default)]
    pub when_profiles: Vec<String>,

    /// Exclude this secret when the active profile has one of these roles.
    /// Use `deny_roles: [install]` to prevent credential files from being mounted
    /// inside install-phase containers where postinstall scripts could read them.
    #[serde(default)]
    pub deny_roles: Vec<ProfileRole>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    Host,
    Sandbox,
}

/// Role of a profile — determines install-style semantics for policy enforcement.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileRole {
    Install,
    Run,
    Build,
}

/// Structured form: `capabilities: { drop: [all], add: [NET_BIND_SERVICE] }`
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CapabilitiesConfig {
    #[serde(default)]
    pub drop: Vec<String>,
    #[serde(default)]
    pub add: Vec<String>,
}

/// Accepts three forms for backward compatibility:
/// - `capabilities: "drop-all"` — keyword string (only "drop-all" is valid)
/// - `capabilities: ["CAP_NET_ADMIN"]` — list treated as cap_add
/// - `capabilities: { drop: [all], add: [NET_BIND_SERVICE] }` — structured (preferred)
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum CapabilitiesSpec {
    Structured(CapabilitiesConfig),
    List(Vec<String>),
    Keyword(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProfileConfig {
    pub mode: ExecutionMode,
    #[serde(default)]
    pub image: Option<ImageConfig>,
    pub network: Option<String>,
    pub writable: Option<bool>,
    pub require_pinned_image: Option<bool>,
    pub require_lockfile: Option<bool>,

    /// Declares what role this profile plays (install, run, build).
    /// `install` enables lockfile auditing and install-style policy enforcement.
    pub role: Option<ProfileRole>,

    /// Lockfile filenames to check when this profile runs an install-style command.
    /// Replaces built-in per-PM lockfile detection.
    #[serde(default)]
    pub lockfile_files: Vec<String>,

    /// Commands to run on the host before the sandboxed command. Each entry is a
    /// shell-quoted command string, e.g. `["npm audit --audit-level=high"]`.
    #[serde(default)]
    pub pre_run: Vec<String>,

    #[serde(default)]
    pub ports: Vec<String>,

    /// When non-empty and `network` is `on`, restrict outbound DNS to only these hostnames.
    /// Implemented by resolving each domain on the host at container-start time and injecting
    /// `--add-host` entries, then pointing the container's DNS at a non-existent server so
    /// arbitrary lookups fail. Raw-IP connections bypass this; package managers use domain names.
    #[serde(default)]
    pub network_allow: Vec<String>,

    pub capabilities: Option<CapabilitiesSpec>,
    pub no_new_privileges: Option<bool>,
    pub read_only_rootfs: Option<bool>,
    pub reuse_container: Option<bool>,
    pub shell: Option<String>,

    /// When set, overrides the workspace-level `writable_paths` for this profile.
    /// Only the listed paths are mounted read-write; all others in the workspace remain read-only.
    pub writable_paths: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DispatchRule {
    #[serde(rename = "match", default)]
    pub patterns: Vec<String>,
    pub profile: String,
}
