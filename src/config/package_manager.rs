use crate::config::model::{
    Config, DispatchRule, ExecutionMode, ProfileConfig, ProfileRole,
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
}

static PRESETS: &[(&str, Preset)] = &[
    (
        "npm",
        Preset {
            install_patterns: &["npm install*", "npm ci*", "npm i *"],
            build_patterns:   &["npm run build*", "npm run compile*"],
            install_writable: &["node_modules", "package-lock.json"],
            build_writable:   &["dist", "node_modules/.vite-temp", "node_modules/.tmp"],
            network_allow:    &["registry.npmjs.org"],
            lockfile_files:   &["package-lock.json", "npm-shrinkwrap.json"],
        },
    ),
    (
        "yarn",
        Preset {
            install_patterns: &["yarn install*", "yarn add*"],
            build_patterns:   &["yarn build*", "yarn run build*"],
            install_writable: &["node_modules", "yarn.lock"],
            build_writable:   &["dist", "node_modules/.vite-temp", "node_modules/.tmp"],
            network_allow:    &["registry.yarnpkg.com", "registry.npmjs.org"],
            lockfile_files:   &["yarn.lock"],
        },
    ),
    (
        "pnpm",
        Preset {
            install_patterns: &["pnpm install*", "pnpm i *", "pnpm add*"],
            build_patterns:   &["pnpm run build*", "pnpm build*"],
            install_writable: &["node_modules", "pnpm-lock.yaml"],
            build_writable:   &["dist", "node_modules/.vite-temp", "node_modules/.tmp"],
            network_allow:    &["registry.npmjs.org"],
            lockfile_files:   &["pnpm-lock.yaml"],
        },
    ),
    (
        "bun",
        Preset {
            install_patterns: &["bun install*", "bun i *", "bun add*"],
            build_patterns:   &["bun build*", "bun run build*"],
            install_writable: &["node_modules", "bun.lockb", "bun.lock"],
            build_writable:   &["dist", "node_modules/.vite-temp", "node_modules/.tmp"],
            network_allow:    &["registry.npmjs.org"],
            lockfile_files:   &["bun.lockb", "bun.lock"],
        },
    ),
    (
        "uv",
        Preset {
            install_patterns: &["uv sync*", "uv add*", "uv pip install*"],
            build_patterns:   &["uv build*", "uv run build*"],
            install_writable: &[".venv"],
            build_writable:   &["dist"],
            network_allow:    &["pypi.org", "files.pythonhosted.org"],
            lockfile_files:   &["uv.lock"],
        },
    ),
    (
        "pip",
        Preset {
            install_patterns: &["pip install*", "pip3 install*"],
            build_patterns:   &[],
            install_writable: &[".venv"],
            build_writable:   &[],
            network_allow:    &["pypi.org", "files.pythonhosted.org"],
            lockfile_files:   &["requirements.txt"],
        },
    ),
    (
        "poetry",
        Preset {
            install_patterns: &["poetry install*", "poetry add*"],
            build_patterns:   &["poetry build*"],
            install_writable: &[".venv"],
            build_writable:   &["dist"],
            network_allow:    &["pypi.org", "files.pythonhosted.org"],
            lockfile_files:   &["poetry.lock"],
        },
    ),
    (
        "cargo",
        Preset {
            // fetch/generate-lockfile acquire deps from the network; build/check/test are offline
            install_patterns: &["cargo fetch*", "cargo generate-lockfile*"],
            build_patterns:   &["cargo build*", "cargo check*", "cargo test*"],
            install_writable: &["target"],
            build_writable:   &["target"],
            network_allow:    &["crates.io", "static.crates.io", "index.crates.io"],
            lockfile_files:   &["Cargo.lock"],
        },
    ),
    (
        "go",
        Preset {
            install_patterns: &["go get*", "go mod download*", "go mod tidy*"],
            // Require explicit -o so the output path is declared and writable
            build_patterns:   &["go build -o dist/*", "go build -o bin/*", "go build ./..."],
            install_writable: &["vendor"],
            build_writable:   &["dist", "bin"],
            network_allow:    &["proxy.golang.org", "sum.golang.org"],
            lockfile_files:   &["go.sum"],
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

fn make_install_profile(_pm_name: &str, preset: &'static Preset, pm: &crate::config::model::PackageManagerConfig) -> ProfileConfig {
    let writable_paths = pm
        .install_writable
        .clone()
        .unwrap_or_else(|| preset.install_writable.iter().map(|s| s.to_string()).collect());

    let network_allow = pm
        .network_allow
        .clone()
        .unwrap_or_else(|| preset.network_allow.iter().map(|s| s.to_string()).collect());

    let pre_run = pm
        .pre_run
        .clone()
        .unwrap_or_default();

    ProfileConfig {
        mode: ExecutionMode::Sandbox,
        image: None,
        network: Some("on".to_string()),
        writable: Some(false),
        require_pinned_image: None,
        require_lockfile: None,
        role: Some(ProfileRole::Install),
        lockfile_files: preset.lockfile_files.iter().map(|s| s.to_string()).collect(),
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

fn make_build_profile(_pm_name: &str, preset: &'static Preset, pm: &crate::config::model::PackageManagerConfig) -> ProfileConfig {
    let writable_paths = pm
        .build_writable
        .clone()
        .unwrap_or_else(|| preset.build_writable.iter().map(|s| s.to_string()).collect());

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
        config.profiles.insert("default".to_string(), make_default_profile());
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
                    patterns: preset.build_patterns.iter().map(|s| s.to_string()).collect(),
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
                patterns: preset.install_patterns.iter().map(|s| s.to_string()).collect(),
                profile: install_profile_key,
            },
        );
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
        assert!(profile.writable_paths.as_deref().unwrap().contains(&"dist".to_string()));
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
            config.profiles.get("pm-npm-install").unwrap().writable_paths.as_deref().unwrap(),
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
}
