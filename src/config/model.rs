#![allow(dead_code)]

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
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceConfig {
    pub root: Option<PathBuf>,
    pub mount: Option<String>,
    pub writable: Option<bool>,
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

    #[serde(default)]
    pub when_profiles: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    Host,
    Sandbox,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ScriptPolicy {
    Allow,
    Ignore,
    Block,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AuditHook {
    NpmAudit,
    PnpmAudit,
    YarnAudit,
    PipAudit,
    CargoAudit,
    BunAudit,
    ComposerAudit,
    GoAudit,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum CapabilitiesSpec {
    Keyword(String),
    List(Vec<String>),
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
    pub script_policy: Option<ScriptPolicy>,

    #[serde(default)]
    pub ports: Vec<String>,

    #[serde(default)]
    pub audit_hooks: Vec<AuditHook>,

    pub capabilities: Option<CapabilitiesSpec>,
    pub no_new_privileges: Option<bool>,
    pub read_only_rootfs: Option<bool>,
    pub reuse_container: Option<bool>,
    pub shell: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DispatchRule {
    #[serde(rename = "match", default)]
    pub patterns: Vec<String>,
    pub profile: String,
}
