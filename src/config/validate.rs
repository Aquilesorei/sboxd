use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::SboxError;

use super::model::{Config, ExecutionMode, ImageConfig, MountType};

pub fn validate_config(config: &Config) -> Result<(), SboxError> {
    let mut errors = Vec::new();
    let mut mount_targets = BTreeSet::new();
    let mut cache_targets = BTreeSet::new();
    let mut secret_names = BTreeSet::new();

    if config.version != 1 {
        errors.push(format!(
            "unsupported config version `{}`; expected `1`",
            config.version
        ));
    }

    match &config.runtime {
        Some(runtime) => {
            if runtime.rootless == Some(false) {
                if let Some(identity) = &config.identity {
                    if identity.map_user == Some(true) {
                        errors.push(
                            "`identity.map_user: true` conflicts with `runtime.rootless: false`; \
                             --userns keep-id is only valid in rootless Podman"
                                .to_string(),
                        );
                    }
                }
            }

            if runtime.require_pinned_image == Some(true) {
                let global_digest_set = config.image.as_ref().and_then(|i| i.digest.as_ref()).is_some();
                if !global_digest_set {
                    let all_sandbox_profiles_have_digest = config.profiles.values()
                        .filter(|p| matches!(p.mode, ExecutionMode::Sandbox))
                        .all(|p| p.image.as_ref().and_then(|i| i.digest.as_ref()).is_some());
                    if !all_sandbox_profiles_have_digest {
                        errors.push(
                            "`runtime.require_pinned_image: true` requires `image.digest` to be set (or all sandbox profiles to have per-profile image digests)".to_string()
                        );
                    }
                }
            }
        }
        None => errors.push("`runtime` section is required".to_string()),
    }

    match &config.workspace {
        Some(workspace) => {
            if workspace.mount.as_deref().is_none_or(str::is_empty) {
                errors.push("`workspace.mount` is required".to_string());
            } else if let Some(mount) = workspace.mount.as_deref()
                && !is_absolute_target(mount)
            {
                errors.push(format!(
                    "`workspace.mount` must be an absolute path: `{mount}`"
                ));
            }

            for path in &workspace.writable_paths {
                if path.trim().is_empty() {
                    errors.push(
                        "`workspace.writable_paths` entries must not be empty".to_string(),
                    );
                    continue;
                }
                let p = std::path::Path::new(path);
                if p.is_absolute() {
                    errors.push(format!(
                        "`workspace.writable_paths` entry must be a relative path: `{path}`"
                    ));
                } else if p
                    .components()
                    .any(|c| c == std::path::Component::ParentDir)
                {
                    errors.push(format!(
                        "`workspace.writable_paths` entry must not contain `..`: `{path}`"
                    ));
                }
            }

            for pattern in &workspace.exclude_paths {
                if pattern.trim().is_empty() {
                    errors.push(
                        "`workspace.exclude_paths` entries must not be empty".to_string(),
                    );
                    continue;
                }
                // Strip the leading **/ glob prefix before checking for absolute paths
                let effective = pattern.trim_start_matches("**/");
                if std::path::Path::new(effective).is_absolute() {
                    errors.push(format!(
                        "`workspace.exclude_paths` entry must be a relative pattern: `{pattern}`"
                    ));
                }
            }
        }
        None => errors.push("`workspace` section is required".to_string()),
    }

    match &config.image {
        Some(image) => validate_image(image, &mut errors),
        None => errors.push("`image` section is required".to_string()),
    }

    if let Some(environment) = &config.environment {
        for name in &environment.pass_through {
            if environment.deny.iter().any(|denied| denied == name) {
                errors.push(format!(
                    "environment variable `{name}` cannot appear in both `pass_through` and `deny`"
                ));
            }
        }
    }

    if config.profiles.is_empty() {
        errors.push("at least one profile must be defined".to_string());
    }

    let mut cache_names = BTreeSet::new();
    for cache in &config.caches {
        if cache.name.trim().is_empty() {
            errors.push("cache names must not be empty".to_string());
        }
        if cache.target.trim().is_empty() {
            errors.push(format!("cache `{}` must define `target`", cache.name));
        } else {
            if !is_absolute_target(&cache.target) {
                errors.push(format!(
                    "cache `{}` target must be absolute: `{}`",
                    cache.name, cache.target
                ));
            }
            if !cache_targets.insert(cache.target.clone()) {
                errors.push(format!("duplicate cache target `{}`", cache.target));
            }
        }
        if !cache_names.insert(cache.name.clone()) {
            errors.push(format!("duplicate cache name `{}`", cache.name));
        }
    }

    for (name, mount) in config.mounts.iter().enumerate() {
        let mount_label = format!("mount #{}", name + 1);

        if mount.target.as_deref().is_none_or(str::is_empty) {
            errors.push(format!("{mount_label} must define `target`"));
        } else if let Some(target) = mount.target.as_deref() {
            if !is_absolute_target(target) {
                errors.push(format!("{mount_label} target must be absolute: `{target}`"));
            }
            if !mount_targets.insert(target.to_string()) {
                errors.push(format!("{mount_label} reuses mount target `{target}`"));
            }
        }

        if matches!(mount.mount_type, MountType::Bind) && mount.source.is_none() {
            errors.push(format!(
                "{mount_label} with `type: bind` must define `source`"
            ));
        }

        if matches!(mount.mount_type, MountType::Tmpfs) && mount.source.is_some() {
            errors.push(format!(
                "{mount_label} with `type: tmpfs` must not define `source`"
            ));
        }

        if let Some(source) = mount.source.as_deref()
            && let Some(message) = validate_mount_source_safety(source)
        {
            errors.push(format!("{mount_label} {message}"));
        }
    }

    for secret in &config.secrets {
        if secret.name.trim().is_empty() {
            errors.push("secret names must not be empty".to_string());
        }
        if secret.source.trim().is_empty() || secret.target.trim().is_empty() {
            errors.push(format!(
                "secret `{}` must define non-empty `source` and `target`",
                secret.name
            ));
        }
        if !secret_names.insert(secret.name.clone()) {
            errors.push(format!("duplicate secret name `{}`", secret.name));
        }
        if !secret.target.trim().is_empty() && !is_absolute_target(&secret.target) {
            errors.push(format!(
                "secret `{}` target must be absolute: `{}`",
                secret.name, secret.target
            ));
        }

        for profile in &secret.when_profiles {
            if !config.profiles.contains_key(profile) {
                errors.push(format!(
                    "secret `{}` references unknown profile `{profile}`",
                    secret.name
                ));
            }
        }
    }

    for (name, profile) in &config.profiles {
        if let Some(image) = &profile.image {
            validate_image(image, &mut errors);
        }

        if matches!(profile.mode, ExecutionMode::Host) && !profile.ports.is_empty() {
            errors.push(format!(
                "profile `{name}` cannot expose ports in `host` mode"
            ));
        }

        if let Some(crate::config::model::CapabilitiesSpec::Keyword(keyword)) =
            &profile.capabilities
        {
            if keyword != "drop-all" {
                errors.push(format!(
                    "profile `{name}` has unknown capabilities keyword `{keyword}`; \
                     use `drop-all`, a list `[CAP_NAME, ...]`, or a structured form `{{ drop: [...], add: [...] }}`"
                ));
            }
        }

        for domain in &profile.network_allow {
            if domain.trim().is_empty() {
                errors.push(format!(
                    "profile `{name}` has an empty entry in `network_allow`"
                ));
            }
        }

        if !profile.network_allow.is_empty() && profile.network.as_deref() == Some("off") {
            errors.push(format!(
                "profile `{name}` sets `network_allow` but `network: off` — allow-listing has no effect when network is disabled"
            ));
        }

        if profile.require_pinned_image == Some(true) {
            let has_digest = profile.image.as_ref().and_then(|i| i.digest.as_ref()).is_some()
                || config.image.as_ref().and_then(|i| i.digest.as_ref()).is_some();
            if !has_digest {
                errors.push(format!(
                    "profile `{name}` sets `require_pinned_image: true` but no image digest is configured (set `image.digest` globally or in the profile's image override)"
                ));
            }
        }
    }

    for (name, rule) in &config.dispatch {
        if rule.patterns.is_empty() {
            errors.push(format!(
                "dispatch rule `{name}` must define at least one pattern"
            ));
        }
        if !config.profiles.contains_key(&rule.profile) {
            errors.push(format!(
                "dispatch rule `{name}` references unknown profile `{}`",
                rule.profile
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(SboxError::ConfigValidation {
            message: errors.join("\n"),
        })
    }
}

/// Collect non-fatal warnings for the config.
/// Returns a Vec so callers can test the warnings without capturing stderr.
pub fn collect_config_warnings(config: &Config) -> Vec<String> {
    let mut warnings = Vec::new();

    // Warn on :latest image references.
    let check_ref = |reference: &str, warnings: &mut Vec<String>| {
        if reference.ends_with(":latest") {
            warnings.push(format!(
                "image `{reference}` uses `:latest` — consider pinning to a specific version \
                 or digest for reproducibility and supply-chain safety"
            ));
        }
    };
    if let Some(image) = &config.image {
        if let Some(r) = &image.reference {
            check_ref(r, &mut warnings);
        }
    }
    for (_name, profile) in &config.profiles {
        if let Some(image) = &profile.image {
            if let Some(r) = &image.reference {
                check_ref(r, &mut warnings);
            }
        }
    }

    // Warn when an install profile uses network: on without network_allow.
    // This gives postinstall scripts unrestricted internet access.
    for (name, profile) in &config.profiles {
        if profile.role == Some(crate::config::model::ProfileRole::Install)
            && profile.network.as_deref() == Some("on")
            && profile.network_allow.is_empty()
        {
            warnings.push(format!(
                "install profile `{name}` uses `network: on` without `network_allow` — \
                 postinstall scripts have unrestricted internet access. \
                 Add `network_allow` to restrict outbound connections to registry hostnames only."
            ));
        }
    }

    // Warn when credential-looking secrets are not restricted from install profiles.
    let has_install_profile = config
        .profiles
        .values()
        .any(|p| p.role == Some(crate::config::model::ProfileRole::Install));

    if has_install_profile {
        for secret in &config.secrets {
            if looks_like_credential(&secret.source)
                && secret.deny_roles.is_empty()
                && secret.when_profiles.is_empty()
            {
                warnings.push(format!(
                    "secret `{}` (source: {}) is not restricted from install profiles — \
                     postinstall scripts can read it. \
                     Add `deny_roles: [install]` to block it from install-phase containers.",
                    secret.name, secret.source
                ));
            }
        }
    }

    warnings
}

/// Print collected warnings to stderr. Called from load_config after validate_config succeeds.
pub fn emit_config_warnings(config: &Config) {
    for warning in collect_config_warnings(config) {
        eprintln!("sbox warning: {warning}");
    }
}

fn looks_like_credential(path: &str) -> bool {
    const PATTERNS: &[&str] = &[
        "npmrc", "netrc", "pypirc", "token", "secret", "credential",
        "id_rsa", "id_ed25519", "id_ecdsa", "id_dsa",
        ".aws/", ".ssh/", "auth.json",
    ];
    let lower = path.to_lowercase();
    PATTERNS.iter().any(|p| lower.contains(p))
}

fn validate_image(image: &ImageConfig, errors: &mut Vec<String>) {
    let source_count =
        image.reference.iter().count() + image.build.iter().count() + image.preset.iter().count();

    match source_count {
        0 => errors
            .push("`image` must define exactly one of `ref`, `build`, or `preset`".to_string()),
        1 => {}
        _ => errors.push(
            "`image.ref`, `image.build`, and `image.preset` are mutually exclusive".to_string(),
        ),
    }

    if let Some(digest) = image.digest.as_deref() {
        if !digest.starts_with("sha256:") {
            errors.push(format!(
                "`image.digest` must start with `sha256:`: `{digest}`"
            ));
        }

        if image.build.is_some() {
            errors.push("`image.digest` cannot be used with `image.build`".to_string());
        }
    }
}

fn is_absolute_target(target: &str) -> bool {
    Path::new(target).is_absolute()
}

fn validate_mount_source_safety(source: &Path) -> Option<String> {
    let source_string = source.to_string_lossy();

    if source_string == "~" || source_string.starts_with("~/") {
        return Some(format!(
            "must not mount home-directory paths implicitly: `{}`",
            source.display()
        ));
    }

    let source = if source.is_absolute() {
        source.to_path_buf()
    } else {
        return None;
    };

    if is_home_root(&source) {
        return Some(format!(
            "must not mount full home-directory roots: `{}`",
            source.display()
        ));
    }

    if is_sensitive_host_path(&source) {
        return Some(format!(
            "must not mount sensitive host credential or socket paths: `{}`",
            source.display()
        ));
    }

    None
}

fn is_home_root(path: &Path) -> bool {
    if let Some(home) = std::env::var_os("HOME")
        && path == Path::new(&home)
    {
        return true;
    }

    matches!(
        path,
        p if p == Path::new("/home")
            || p == Path::new("/root")
            || p == Path::new("/Users")
    )
}

fn is_sensitive_host_path(path: &Path) -> bool {
    const EXACT_PATHS: &[&str] = &[
        "/var/run/docker.sock",
        "/run/docker.sock",
        "/var/run/podman/podman.sock",
        "/run/podman/podman.sock",
    ];
    const PREFIX_PATHS: &[&str] = &[".ssh", ".aws", ".kube", ".config/gcloud", ".gnupg", ".docker"];
    const FILE_PATHS: &[&str] = &[
        ".git-credentials",
        ".npmrc",
        ".pypirc",
        ".netrc",
        ".docker/config.json",
    ];

    if EXACT_PATHS
        .iter()
        .any(|candidate| path == Path::new(candidate))
    {
        return true;
    }

    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if PREFIX_PATHS
            .iter()
            .map(|suffix| home.join(suffix))
            .any(|candidate| path == candidate)
        {
            return true;
        }
        if FILE_PATHS
            .iter()
            .map(|suffix| home.join(suffix))
            .any(|candidate| path == candidate)
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::validate_config;
    use crate::config::model::{
        BackendKind, Config, DispatchRule, EnvironmentConfig, ExecutionMode, ImageConfig,
        MountConfig, MountType, ProfileConfig, RuntimeConfig, WorkspaceConfig,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn base_config() -> Config {
        let mut profiles = IndexMap::new();
        profiles.insert(
            "default".to_string(),
            ProfileConfig {
                mode: ExecutionMode::Sandbox,
                image: None,
                network: Some("off".into()),
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
                root: Some(PathBuf::from(".")),
                mount: Some("/workspace".into()),
                writable: Some(true),
                writable_paths: Vec::new(),
                exclude_paths: Vec::new(),
            }),
            identity: None,
            image: Some(ImageConfig {
                reference: Some("python:3.13-slim".into()),
                build: None,
                preset: None,
                digest: None,
                verify_signature: None,
                pull_policy: None,
                tag: None,
            }),
            environment: Some(EnvironmentConfig {
                pass_through: Vec::new(),
                set: BTreeMap::new(),
                deny: Vec::new(),
            }),
            mounts: Vec::new(),
            caches: Vec::new(),
            secrets: Vec::new(),
            profiles,
            dispatch: IndexMap::<String, DispatchRule>::new(),
            package_manager: None,
        }
    }

    #[test]
    fn rejects_overlapping_pass_through_and_deny_variables() {
        let mut config = base_config();
        config.environment = Some(EnvironmentConfig {
            pass_through: vec!["SSH_AUTH_SOCK".into()],
            set: BTreeMap::new(),
            deny: vec!["SSH_AUTH_SOCK".into()],
        });

        let error = validate_config(&config).expect_err("validation should fail");
        assert!(
            error
                .to_string()
                .contains("cannot appear in both `pass_through` and `deny`")
        );
    }

    #[test]
    fn rejects_dangerous_docker_socket_mounts() {
        let mut config = base_config();
        config.mounts.push(MountConfig {
            source: Some(PathBuf::from("/var/run/docker.sock")),
            target: Some("/run/docker.sock".into()),
            mount_type: MountType::Bind,
            read_only: Some(true),
            create: None,
        });

        let error = validate_config(&config).expect_err("validation should fail");
        assert!(
            error
                .to_string()
                .contains("must not mount sensitive host credential or socket paths")
        );
    }

    #[test]
    fn rejects_docker_config_dir_mount() {
        let home = std::env::var("HOME").expect("HOME must be set");
        let mut config = base_config();
        config.mounts.push(MountConfig {
            source: Some(PathBuf::from(format!("{home}/.docker"))),
            target: Some("/run/docker-config".into()),
            mount_type: MountType::Bind,
            read_only: Some(true),
            create: None,
        });

        let error = validate_config(&config).expect_err("validation should fail");
        assert!(
            error
                .to_string()
                .contains("must not mount sensitive host credential or socket paths")
        );
    }

    #[test]
    fn rejects_docker_config_json_mount() {
        let home = std::env::var("HOME").expect("HOME must be set");
        let mut config = base_config();
        config.mounts.push(MountConfig {
            source: Some(PathBuf::from(format!("{home}/.docker/config.json"))),
            target: Some("/run/docker-config.json".into()),
            mount_type: MountType::Bind,
            read_only: Some(true),
            create: None,
        });

        let error = validate_config(&config).expect_err("validation should fail");
        assert!(
            error
                .to_string()
                .contains("must not mount sensitive host credential or socket paths")
        );
    }
}
