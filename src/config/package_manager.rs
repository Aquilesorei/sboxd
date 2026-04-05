use crate::config::model::{
    Config, DispatchRule, EnvironmentConfig, ExecutionMode, ImageConfig, ProfileConfig,
    ProfileRole, WorkspaceConfig,
};
use crate::error::SboxError;

/// Built-in package manager preset.
struct Preset {
    install_patterns: &'static [&'static str],
    build_patterns: &'static [&'static str],
    install_writable: &'static [&'static str],
    build_writable: &'static [&'static str],
    network_allow: &'static [&'static str],
    lockfile_files: &'static [&'static str],
    /// Environment variables that carry publish credentials for this registry.
    /// These are automatically added to `environment.deny` so a worm running
    /// inside the install container cannot use them to publish new packages —
    /// even though the registry hostname is in the network allow-list.
    /// (Shai-Hulud worm, Sept–Dec 2025: stole tokens and auto-published via allowed registry.)
    publish_token_envs: &'static [&'static str],
    /// Credential files that should be excluded from the workspace during installs.
    /// Added to `workspace.exclude_paths` automatically by the preset.
    credential_files: &'static [&'static str],
    /// Default container image for this ecosystem.
    /// Injected into `image:` when the user omits the section entirely, so a minimal
    /// `sbox.yaml` only needs `package_manager: name: <pm>` — no image knowledge required.
    default_image: &'static str,
    /// Default env vars injected when the user omits `environment.set`, specific to this
    /// ecosystem's toolchain (e.g. disable managed Python downloads inside a uv container).
    default_env: &'static [(&'static str, &'static str)],
}

static PRESETS: &[(&str, Preset)] = &[
    (
        "npm",
        Preset {
            install_patterns: &["npm install*", "npm ci*", "npm i *"],
            build_patterns: &["npm run build*", "npm run compile*"],
            install_writable: &["node_modules", "package-lock.json"],
            build_writable: &["dist", "node_modules/.vite-temp", "node_modules/.tmp"],
            network_allow: &["registry.npmjs.org"],
            lockfile_files: &["package-lock.json", "npm-shrinkwrap.json"],
            publish_token_envs: &["NPM_TOKEN", "NODE_AUTH_TOKEN", "NPM_AUTH_TOKEN"],
            credential_files: &[".npmrc"],
            default_image: "node:22-bookworm-slim",
            default_env: &[],
        },
    ),
    (
        "yarn",
        Preset {
            install_patterns: &["yarn install*", "yarn add*"],
            build_patterns: &["yarn build*", "yarn run build*"],
            install_writable: &["node_modules", "yarn.lock"],
            build_writable: &["dist", "node_modules/.vite-temp", "node_modules/.tmp"],
            network_allow: &["registry.yarnpkg.com", "registry.npmjs.org"],
            lockfile_files: &["yarn.lock"],
            publish_token_envs: &[
                "NPM_TOKEN",
                "NODE_AUTH_TOKEN",
                "NPM_AUTH_TOKEN",
                "YARN_AUTH_TOKEN",
            ],
            credential_files: &[".npmrc", ".yarnrc.yml"],
            default_image: "node:22-bookworm-slim",
            default_env: &[],
        },
    ),
    (
        "pnpm",
        Preset {
            install_patterns: &["pnpm install*", "pnpm i *", "pnpm add*"],
            build_patterns: &["pnpm run build*", "pnpm build*"],
            install_writable: &["node_modules", "pnpm-lock.yaml"],
            build_writable: &["dist", "node_modules/.vite-temp", "node_modules/.tmp"],
            network_allow: &["registry.npmjs.org"],
            lockfile_files: &["pnpm-lock.yaml"],
            publish_token_envs: &["NPM_TOKEN", "NODE_AUTH_TOKEN", "NPM_AUTH_TOKEN"],
            credential_files: &[".npmrc"],
            default_image: "node:22-bookworm-slim",
            default_env: &[],
        },
    ),
    (
        "bun",
        Preset {
            install_patterns: &["bun install*", "bun i *", "bun add*"],
            build_patterns: &["bun build*", "bun run build*"],
            install_writable: &["node_modules", "bun.lockb", "bun.lock"],
            build_writable: &["dist", "node_modules/.vite-temp", "node_modules/.tmp"],
            network_allow: &["registry.npmjs.org"],
            lockfile_files: &["bun.lockb", "bun.lock"],
            publish_token_envs: &["NPM_TOKEN", "NODE_AUTH_TOKEN", "NPM_AUTH_TOKEN"],
            credential_files: &[".npmrc"],
            default_image: "oven/bun:1",
            default_env: &[],
        },
    ),
    (
        "uv",
        Preset {
            install_patterns: &["uv sync*", "uv add*", "uv pip install*"],
            build_patterns: &["uv build*", "uv run build*"],
            install_writable: &[".venv"],
            build_writable: &["dist"],
            network_allow: &["pypi.org", "files.pythonhosted.org"],
            lockfile_files: &["uv.lock"],
            publish_token_envs: &["UV_PUBLISH_TOKEN", "TWINE_PASSWORD", "PYPI_TOKEN"],
            credential_files: &[".pypirc"],
            default_image: "ghcr.io/astral-sh/uv:python3.13-bookworm-slim",
            // Prevent uv from downloading a managed Python interpreter inside the sandbox;
            // the image already ships Python. Without this uv tries github.com which is not
            // in the network_allow list and fails with a confusing DNS error.
            default_env: &[("UV_PYTHON_DOWNLOADS", "never")],
        },
    ),
    (
        "pip",
        Preset {
            install_patterns: &["pip install*", "pip3 install*"],
            build_patterns: &[],
            install_writable: &[".venv"],
            build_writable: &[],
            network_allow: &["pypi.org", "files.pythonhosted.org"],
            lockfile_files: &["requirements.txt"],
            publish_token_envs: &["TWINE_PASSWORD", "TWINE_USERNAME", "PYPI_TOKEN"],
            credential_files: &[".pypirc"],
            default_image: "python:3.13-slim",
            default_env: &[],
        },
    ),
    (
        "poetry",
        Preset {
            install_patterns: &["poetry install*", "poetry add*"],
            build_patterns: &["poetry build*"],
            install_writable: &[".venv"],
            build_writable: &["dist"],
            network_allow: &["pypi.org", "files.pythonhosted.org"],
            lockfile_files: &["poetry.lock"],
            publish_token_envs: &["POETRY_PYPI_TOKEN_PYPI", "TWINE_PASSWORD", "PYPI_TOKEN"],
            credential_files: &[".pypirc"],
            default_image: "python:3.13-slim",
            default_env: &[],
        },
    ),
    (
        "cargo",
        Preset {
            // fetch/generate-lockfile acquire deps from the network; build/check/test are offline
            install_patterns: &["cargo fetch*", "cargo generate-lockfile*"],
            build_patterns: &["cargo build*", "cargo check*", "cargo test*"],
            install_writable: &["target"],
            build_writable: &["target"],
            network_allow: &["crates.io", "static.crates.io", "index.crates.io"],
            lockfile_files: &["Cargo.lock"],
            publish_token_envs: &["CARGO_REGISTRY_TOKEN"],
            // ~/.cargo/credentials is home-dir and already blocked; no workspace credential file
            credential_files: &[],
            default_image: "rust:1-bookworm",
            default_env: &[],
        },
    ),
    (
        "go",
        Preset {
            install_patterns: &["go get*", "go mod download*", "go mod tidy*"],
            // go build ./... writes the binary to CWD by default; "." must be writable.
            // Network is off on the build profile so this is lower risk than on install.
            build_patterns: &[
                "go build -o dist/*",
                "go build -o bin/*",
                "go build ./...",
                "go build*",
            ],
            install_writable: &["vendor"],
            build_writable: &["dist", "bin", "."],
            network_allow: &["proxy.golang.org", "sum.golang.org"],
            lockfile_files: &["go.sum"],
            // Go module proxy is read-only; no publish token concept
            publish_token_envs: &[],
            credential_files: &[],
            default_image: "golang:1.23-bookworm",
            default_env: &[],
        },
    ),
];

fn lookup_preset(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|(n, _)| *n == name).map(|(_, p)| p)
}

/// Dispatch rule key encoding — encodes pm name and kind so plan.rs can render a friendly string.
/// Format: `pm:<name>:<kind>` e.g. `pm:npm:install`
fn dispatch_key(pm_name: &str, kind: &str) -> String {
    format!("pm:{pm_name}:{kind}")
}

fn make_install_profile(
    _pm_name: &str,
    preset: &'static Preset,
    pm: &crate::config::model::PackageManagerConfig,
) -> ProfileConfig {
    let writable_paths = pm.install_writable.clone().unwrap_or_else(|| {
        preset
            .install_writable
            .iter()
            .map(|s| s.to_string())
            .collect()
    });

    let network_allow = pm
        .network_allow
        .clone()
        .unwrap_or_else(|| preset.network_allow.iter().map(|s| s.to_string()).collect());

    let pre_run = pm.pre_run.clone().unwrap_or_default();

    ProfileConfig {
        mode: ExecutionMode::Sandbox,
        image: None,
        network: Some("on".to_string()),
        writable: Some(false),
        require_pinned_image: None,
        require_lockfile: None,
        role: Some(ProfileRole::Install),
        lockfile_files: preset
            .lockfile_files
            .iter()
            .map(|s| s.to_string())
            .collect(),
        pre_run,
        ports: Vec::new(),
        network_allow,
        capabilities: None,
        no_new_privileges: Some(true),
        read_only_rootfs: None,
        reuse_container: None,
        shell: None,
        writable_paths: Some(writable_paths),
    }
}

fn make_build_profile(
    _pm_name: &str,
    preset: &'static Preset,
    pm: &crate::config::model::PackageManagerConfig,
) -> ProfileConfig {
    let writable_paths = pm.build_writable.clone().unwrap_or_else(|| {
        preset
            .build_writable
            .iter()
            .map(|s| s.to_string())
            .collect()
    });

    ProfileConfig {
        mode: ExecutionMode::Sandbox,
        image: None,
        network: Some("off".to_string()),
        writable: Some(false),
        require_pinned_image: None,
        require_lockfile: None,
        role: Some(ProfileRole::Build),
        lockfile_files: Vec::new(),
        pre_run: Vec::new(),
        ports: Vec::new(),
        network_allow: Vec::new(),
        capabilities: None,
        no_new_privileges: Some(true),
        read_only_rootfs: None,
        reuse_container: None,
        shell: None,
        writable_paths: Some(writable_paths),
    }
}

fn make_default_profile() -> ProfileConfig {
    ProfileConfig {
        mode: ExecutionMode::Sandbox,
        image: None,
        network: Some("off".to_string()),
        writable: Some(false),
        require_pinned_image: None,
        require_lockfile: None,
        role: Some(ProfileRole::Run),
        lockfile_files: Vec::new(),
        pre_run: Vec::new(),
        ports: Vec::new(),
        network_allow: Vec::new(),
        capabilities: None,
        no_new_privileges: Some(true),
        read_only_rootfs: None,
        reuse_container: None,
        shell: None,
        writable_paths: Some(vec![]),
    }
}

/// Elaborates `package_manager:` into synthetic profiles and dispatch rules.
/// Called after deserialization, before validation.
/// User-defined profiles and dispatch rules always take precedence.
///
/// Also fills in `image:` and `workspace:` defaults when the user omits them,
/// so a minimal config only needs `package_manager: name: <pm>`.
pub fn elaborate(config: &mut Config) -> Result<(), SboxError> {
    let pm = match config.package_manager.as_ref() {
        Some(pm) => pm.clone(),
        None => return Ok(()),
    };

    let preset = lookup_preset(&pm.name).ok_or_else(|| SboxError::ConfigValidation {
        message: format!(
            "unknown package_manager name `{}`; valid names: npm, yarn, pnpm, bun, uv, pip, poetry, cargo, go",
            pm.name
        ),
    })?;

    // Inject a default (auto-detect) runtime when omitted — backend detection handles the rest.
    if config.runtime.is_none() {
        config.runtime = Some(crate::config::model::RuntimeConfig {
            backend: None,
            rootless: None,
            strict_security: None,
            reuse_container: None,
            container_name: None,
            pull_policy: None,
            require_pinned_image: None,
        });
    }

    // Inject image default when omitted entirely — user doesn't need to know the right image.
    if config.image.is_none() {
        config.image = Some(ImageConfig {
            reference: Some(preset.default_image.to_string()),
            digest: None,
            verify_signature: None,
            build: None,
            preset: None,
            pull_policy: None,
            tag: None,
        });
    }

    // Inject workspace defaults — /workspace mount with safe read-only default.
    if config.workspace.is_none() {
        config.workspace = Some(WorkspaceConfig {
            root: None,
            mount: Some("/workspace".to_string()),
            writable: Some(false),
            writable_paths: Vec::new(),
            exclude_paths: Vec::new(),
        });
    }

    // Inject toolchain-specific env defaults (e.g. UV_PYTHON_DOWNLOADS=never).
    // Only add keys not already explicitly set by the user.
    if !preset.default_env.is_empty() {
        let env = config
            .environment
            .get_or_insert_with(EnvironmentConfig::default);
        for &(key, value) in preset.default_env {
            env.set
                .entry(key.to_string())
                .or_insert_with(|| value.to_string());
        }
    }

    let pm_name = pm.name.clone();

    // Synthesize profiles (only insert if the user hasn't defined them already).
    let install_profile_key = format!("pm-{pm_name}-install");
    let build_profile_key = format!("pm-{pm_name}-build");

    if !config.profiles.contains_key(&install_profile_key) {
        config.profiles.insert(
            install_profile_key.clone(),
            make_install_profile(&pm_name, preset, &pm),
        );
    }

    if !config.profiles.contains_key(&build_profile_key) && !preset.build_patterns.is_empty() {
        config.profiles.insert(
            build_profile_key.clone(),
            make_build_profile(&pm_name, preset, &pm),
        );
    }

    // Synthesize the default profile only if the user hasn't defined one.
    if !config.profiles.contains_key("default") {
        config
            .profiles
            .insert("default".to_string(), make_default_profile());
    }

    // Prepend dispatch rules so user-defined rules take precedence (IndexMap preserves order).
    // We insert in reverse order so they end up in the right sequence after shift_insert(0, ...).
    if !preset.build_patterns.is_empty() {
        let build_key = dispatch_key(&pm_name, "build");
        if !config.dispatch.contains_key(&build_key) {
            config.dispatch.shift_insert(
                0,
                build_key,
                DispatchRule {
                    patterns: preset
                        .build_patterns
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    profile: build_profile_key,
                },
            );
        }
    }

    let install_key = dispatch_key(&pm_name, "install");
    if !config.dispatch.contains_key(&install_key) {
        config.dispatch.shift_insert(
            0,
            install_key,
            DispatchRule {
                patterns: preset
                    .install_patterns
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                profile: install_profile_key,
            },
        );
    }

    // Inject publish-token denials so a worm inside the install container cannot use
    // the allowed registry endpoint to publish new poisoned packages.
    // Only add tokens not already explicitly listed — user deny lists take precedence.
    if !preset.publish_token_envs.is_empty() {
        let env = config
            .environment
            .get_or_insert_with(EnvironmentConfig::default);
        for &token_var in preset.publish_token_envs {
            if !env.deny.iter().any(|d| d == token_var) {
                env.deny.push(token_var.to_string());
            }
        }
    }

    // Inject credential file exclusions so workspace-level auth files are masked inside
    // the install container even if the user hasn't listed them explicitly.
    if !preset.credential_files.is_empty()
        && let Some(workspace) = config.workspace.as_mut()
    {
        for &cred_file in preset.credential_files {
            if !workspace.exclude_paths.iter().any(|p| p == cred_file) {
                workspace.exclude_paths.push(cred_file.to_string());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::PackageManagerConfig;

    fn base_config_with_pm(name: &str) -> Config {
        Config {
            version: 1,
            runtime: None,
            workspace: Some(crate::config::model::WorkspaceConfig {
                root: None,
                mount: Some("/workspace".to_string()),
                writable: Some(false),
                writable_paths: vec![],
                exclude_paths: vec![],
            }),
            identity: None,
            image: Some(crate::config::model::ImageConfig {
                reference: Some("node:22-bookworm-slim".to_string()),
                digest: None,
                verify_signature: None,
                build: None,
                preset: None,
                pull_policy: None,
                tag: None,
            }),
            environment: None,
            mounts: vec![],
            caches: vec![],
            secrets: vec![],
            profiles: indexmap::IndexMap::new(),
            dispatch: indexmap::IndexMap::new(),
            package_manager: Some(PackageManagerConfig {
                name: name.to_string(),
                install_writable: None,
                build_writable: None,
                network_allow: None,
                pre_run: None,
            }),
        }
    }

    #[test]
    fn test_npm_generates_install_profile() {
        let mut config = base_config_with_pm("npm");
        elaborate(&mut config).unwrap();

        let profile = config.profiles.get("pm-npm-install").unwrap();
        assert_eq!(profile.network.as_deref(), Some("on"));
        assert_eq!(profile.writable, Some(false));
        assert_eq!(
            profile.writable_paths.as_deref().unwrap(),
            &["node_modules", "package-lock.json"]
        );
        assert_eq!(profile.role, Some(ProfileRole::Install));
    }

    #[test]
    fn test_npm_generates_build_profile() {
        let mut config = base_config_with_pm("npm");
        elaborate(&mut config).unwrap();

        let profile = config.profiles.get("pm-npm-build").unwrap();
        assert_eq!(profile.network.as_deref(), Some("off"));
        assert!(
            profile
                .writable_paths
                .as_deref()
                .unwrap()
                .contains(&"dist".to_string())
        );
    }

    #[test]
    fn test_default_profile_inserted_when_absent() {
        let mut config = base_config_with_pm("npm");
        elaborate(&mut config).unwrap();
        assert!(config.profiles.contains_key("default"));
        let default = config.profiles.get("default").unwrap();
        assert_eq!(default.network.as_deref(), Some("off"));
        assert_eq!(default.writable_paths.as_deref().unwrap(), &[] as &[String]);
    }

    #[test]
    fn test_default_profile_not_overridden() {
        let mut config = base_config_with_pm("npm");
        // Pre-insert a user default profile with network on
        config.profiles.insert(
            "default".to_string(),
            ProfileConfig {
                mode: ExecutionMode::Sandbox,
                image: None,
                network: Some("on".to_string()),
                writable: Some(true),
                require_pinned_image: None,
                require_lockfile: None,
                role: None,
                lockfile_files: vec![],
                pre_run: vec![],
                ports: vec![],
                network_allow: vec![],
                capabilities: None,
                no_new_privileges: None,
                read_only_rootfs: None,
                reuse_container: None,
                shell: None,
                writable_paths: None,
            },
        );
        elaborate(&mut config).unwrap();
        // User's default should be preserved
        assert_eq!(
            config.profiles.get("default").unwrap().network.as_deref(),
            Some("on")
        );
    }

    #[test]
    fn test_dispatch_rules_prepended() {
        let mut config = base_config_with_pm("npm");
        // Pre-insert a user dispatch rule
        config.dispatch.insert(
            "my-rule".to_string(),
            DispatchRule {
                patterns: vec!["npm install --my-custom*".to_string()],
                profile: "default".to_string(),
            },
        );
        elaborate(&mut config).unwrap();
        // PM rules must be at indices 0 and 1, user rule at end
        let keys: Vec<&str> = config.dispatch.keys().map(|s| s.as_str()).collect();
        assert!(keys[0].starts_with("pm:npm:"));
        assert!(keys[1].starts_with("pm:npm:"));
        assert_eq!(*keys.last().unwrap(), "my-rule");
    }

    #[test]
    fn test_unknown_pm_name_errors() {
        let mut config = base_config_with_pm("gradle");
        let err = elaborate(&mut config).unwrap_err();
        assert!(matches!(err, SboxError::ConfigValidation { .. }));
    }

    #[test]
    fn test_install_writable_override() {
        let mut config = base_config_with_pm("npm");
        config.package_manager.as_mut().unwrap().install_writable =
            Some(vec!["node_modules".to_string(), ".cache".to_string()]);
        elaborate(&mut config).unwrap();
        assert_eq!(
            config
                .profiles
                .get("pm-npm-install")
                .unwrap()
                .writable_paths
                .as_deref()
                .unwrap(),
            &["node_modules", ".cache"]
        );
    }

    #[test]
    fn test_no_pm_is_noop() {
        let mut config = base_config_with_pm("npm");
        config.package_manager = None;
        elaborate(&mut config).unwrap();
        assert!(config.profiles.is_empty());
        assert!(config.dispatch.is_empty());
    }

    #[test]
    fn test_npm_injects_publish_token_denials() {
        let mut config = base_config_with_pm("npm");
        elaborate(&mut config).unwrap();

        let deny = &config.environment.as_ref().unwrap().deny;
        assert!(deny.contains(&"NPM_TOKEN".to_string()));
        assert!(deny.contains(&"NODE_AUTH_TOKEN".to_string()));
        assert!(deny.contains(&"NPM_AUTH_TOKEN".to_string()));
    }

    #[test]
    fn test_npm_does_not_duplicate_existing_deny() {
        let mut config = base_config_with_pm("npm");
        config.environment = Some(crate::config::model::EnvironmentConfig {
            pass_through: vec![],
            set: std::collections::BTreeMap::new(),
            deny: vec!["NPM_TOKEN".to_string()],
        });
        elaborate(&mut config).unwrap();

        let deny = &config.environment.as_ref().unwrap().deny;
        assert_eq!(deny.iter().filter(|d| *d == "NPM_TOKEN").count(), 1);
    }

    #[test]
    fn test_npm_injects_credential_file_exclusion() {
        let mut config = base_config_with_pm("npm");
        elaborate(&mut config).unwrap();

        let exclude = &config.workspace.as_ref().unwrap().exclude_paths;
        assert!(exclude.contains(&".npmrc".to_string()));
    }

    #[test]
    fn test_npm_does_not_duplicate_existing_exclude() {
        let mut config = base_config_with_pm("npm");
        config.workspace.as_mut().unwrap().exclude_paths = vec![".npmrc".to_string()];
        elaborate(&mut config).unwrap();

        let exclude = &config.workspace.as_ref().unwrap().exclude_paths;
        assert_eq!(exclude.iter().filter(|p| *p == ".npmrc").count(), 1);
    }

    #[test]
    fn test_go_has_no_publish_token_denials() {
        let mut config = base_config_with_pm("go");
        config.image = Some(crate::config::model::ImageConfig {
            reference: Some("golang:1.23-bookworm".to_string()),
            digest: None,
            verify_signature: None,
            build: None,
            preset: None,
            pull_policy: None,
            tag: None,
        });
        elaborate(&mut config).unwrap();
        // Go has no publish tokens — environment should not be created just for denials
        if let Some(env) = &config.environment {
            assert!(env.deny.is_empty());
        }
    }

    #[test]
    fn test_cargo_injects_registry_token_denial() {
        let mut config = base_config_with_pm("cargo");
        config.image = Some(crate::config::model::ImageConfig {
            reference: Some("rust:1-bookworm".to_string()),
            digest: None,
            verify_signature: None,
            build: None,
            preset: None,
            pull_policy: None,
            tag: None,
        });
        elaborate(&mut config).unwrap();

        let deny = &config.environment.as_ref().unwrap().deny;
        assert!(deny.contains(&"CARGO_REGISTRY_TOKEN".to_string()));
    }
}
