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
            if runtime.backend.is_none() {
                errors.push("`runtime.backend` is required".to_string());
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
    const PREFIX_PATHS: &[&str] = &[".ssh", ".aws", ".kube", ".config/gcloud", ".gnupg"];
    const FILE_PATHS: &[&str] = &[".git-credentials", ".npmrc", ".pypirc", ".netrc"];

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
                root: Some(PathBuf::from(".")),
                mount: Some("/workspace".into()),
                writable: Some(true),
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
}
