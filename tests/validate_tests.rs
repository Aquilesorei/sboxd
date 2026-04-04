use indexmap::IndexMap;
use std::collections::BTreeMap;
use std::path::PathBuf;

use sbox::config::model::{
    BackendKind, CacheConfig, CapabilitiesSpec, Config, DispatchRule, EnvironmentConfig,
    ExecutionMode, IdentityConfig, ImageConfig, MountConfig, MountType, ProfileConfig,
    RuntimeConfig, WorkspaceConfig,
};
use sbox::config::validate::validate_config;

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

    Config {
        version: 1,
        runtime: Some(RuntimeConfig {
            backend: Some(BackendKind::Podman),
            rootless: Some(true),
            strict_security: None,
            reuse_container: Some(false),
            container_name: None,
            pull_policy: None,
            require_pinned_image: None,
        }),
        workspace: Some(WorkspaceConfig {
            root: Some(PathBuf::from(".")),
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
        environment: Some(EnvironmentConfig {
            pass_through: Vec::new(),
            set: BTreeMap::new(),
            deny: Vec::new(),
        }),
        mounts: Vec::new(),
        caches: Vec::new(),
        secrets: Vec::new(),
        profiles,
        dispatch: IndexMap::new(),

        package_manager: None,
    }
}

#[test]
fn accepts_valid_minimal_config() {
    let config = base_config();
    validate_config(&config).expect("valid config should pass");
}

#[test]
fn rejects_unsupported_config_version() {
    let mut config = base_config();
    config.version = 99;

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("unsupported config version"));
}

#[test]
fn rejects_missing_runtime_section() {
    let mut config = base_config();
    config.runtime = None;

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("runtime"));
}

#[test]
fn accepts_missing_runtime_backend() {
    // backend is now optional — auto-detection handles the missing case at runtime
    let mut config = base_config();
    config.runtime.as_mut().unwrap().backend = None;

    validate_config(&config).expect("missing backend should be accepted (auto-detected at runtime)");
}

#[test]
fn rejects_missing_workspace_section() {
    let mut config = base_config();
    config.workspace = None;

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("workspace"));
}

#[test]
fn rejects_missing_workspace_mount() {
    let mut config = base_config();
    config.workspace.as_mut().unwrap().mount = None;

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("mount"));
}

#[test]
fn rejects_relative_workspace_mount() {
    let mut config = base_config();
    config.workspace.as_mut().unwrap().mount = Some("relative/path".to_string());

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("absolute path"));
}

#[test]
fn rejects_missing_image_section() {
    let mut config = base_config();
    config.image = None;

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("image"));
}

#[test]
fn rejects_missing_profile() {
    let mut config = base_config();
    config.profiles.clear();

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("profile"));
}

#[test]
fn rejects_host_profile_with_ports() {
    let mut config = base_config();
    let mut host_profile = config.profiles.get_mut("default").unwrap().clone();
    host_profile.mode = ExecutionMode::Host;
    host_profile.ports = vec!["8080:80".to_string()];
    config.profiles.insert("host".to_string(), host_profile);

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("ports"));
    assert!(error.to_string().contains("cannot expose ports"));
}

#[test]
fn accepts_valid_cache_with_absolute_target() {
    let mut config = base_config();
    config.caches.push(CacheConfig {
        name: "uv-cache".to_string(),
        target: "/root/.cache/uv".to_string(),
        source: None,
        read_only: Some(true),
    });

    validate_config(&config).expect("valid cache should pass");
}

#[test]
fn rejects_cache_with_relative_target() {
    let mut config = base_config();
    config.caches.push(CacheConfig {
        name: "cache".to_string(),
        target: "relative/path".to_string(),
        source: None,
        read_only: None,
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("target must be absolute"));
}

#[test]
fn rejects_cache_with_empty_name() {
    let mut config = base_config();
    config.caches.push(CacheConfig {
        name: "".to_string(),
        target: "/cache".to_string(),
        source: None,
        read_only: None,
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("empty"));
}

#[test]
fn rejects_duplicate_cache_targets() {
    let mut config = base_config();
    config.caches.push(CacheConfig {
        name: "cache1".to_string(),
        target: "/cache/shared".to_string(),
        source: None,
        read_only: None,
    });
    config.caches.push(CacheConfig {
        name: "cache2".to_string(),
        target: "/cache/shared".to_string(),
        source: None,
        read_only: None,
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("duplicate cache target"));
}

#[test]
fn accepts_valid_bind_mount() {
    let mut config = base_config();
    config.mounts.push(MountConfig {
        source: Some(PathBuf::from("/tmp/data")),
        target: Some("/data".to_string()),
        mount_type: MountType::Bind,
        read_only: Some(true),
        create: None,
    });

    validate_config(&config).expect("valid bind mount should pass");
}

#[test]
fn rejects_bind_mount_without_source() {
    let mut config = base_config();
    config.mounts.push(MountConfig {
        source: None,
        target: Some("/tmpfs".to_string()),
        mount_type: MountType::Bind,
        read_only: None,
        create: None,
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("bind"));
    assert!(error.to_string().contains("source"));
}

#[test]
fn rejects_tmpfs_mount_with_source() {
    let mut config = base_config();
    config.mounts.push(MountConfig {
        source: Some(PathBuf::from("/tmp/data")),
        target: Some("/tmpfs".to_string()),
        mount_type: MountType::Tmpfs,
        read_only: None,
        create: None,
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("tmpfs"));
    assert!(error.to_string().contains("source"));
}

#[test]
fn accepts_tmpfs_mount_without_source() {
    let mut config = base_config();
    config.mounts.push(MountConfig {
        source: None,
        target: Some("/tmpfs".to_string()),
        mount_type: MountType::Tmpfs,
        read_only: None,
        create: None,
    });

    validate_config(&config).expect("valid tmpfs mount should pass");
}

#[test]
fn rejects_mount_to_relative_path() {
    let mut config = base_config();
    config.mounts.push(MountConfig {
        source: Some(PathBuf::from("/tmp/data")),
        target: Some("relative".to_string()),
        mount_type: MountType::Bind,
        read_only: None,
        create: None,
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("must be absolute"));
}

#[test]
fn rejects_podman_socket_mount() {
    let mut config = base_config();
    config.mounts.push(MountConfig {
        source: Some(PathBuf::from("/var/run/podman/podman.sock")),
        target: Some("/var/run/podman/podman.sock".to_string()),
        mount_type: MountType::Bind,
        read_only: Some(true),
        create: None,
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("sensitive"));
}

#[test]
fn rejects_ssh_directory_mount() {
    let mut config = base_config();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/user".to_string());
    config.mounts.push(MountConfig {
        source: Some(PathBuf::from(format!("{}/.ssh", home))),
        target: Some("/ssh".to_string()),
        mount_type: MountType::Bind,
        read_only: Some(true),
        create: None,
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("sensitive"));
}

#[test]
fn rejects_kube_directory_mount() {
    let mut config = base_config();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/user".to_string());
    config.mounts.push(MountConfig {
        source: Some(PathBuf::from(format!("{}/.kube", home))),
        target: Some("/kube".to_string()),
        mount_type: MountType::Bind,
        read_only: Some(true),
        create: None,
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("sensitive"));
}

#[test]
fn accepts_valid_secret() {
    let mut config = base_config();
    config.profiles.insert(
        "install".to_string(),
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
    config.secrets.push(sbox::config::model::SecretConfig {
        name: "npm_token".to_string(),
        source: format!(
            "{}/.npmrc",
            std::env::var("HOME").unwrap_or_else(|_| "/home/user".to_string())
        ),
        target: "/run/secrets/npm_token".to_string(),
        when_profiles: vec!["install".to_string()],
    });

    validate_config(&config).expect("valid secret should pass");
}

#[test]
fn rejects_secret_referencing_unknown_profile() {
    let mut config = base_config();
    config.secrets.push(sbox::config::model::SecretConfig {
        name: "token".to_string(),
        source: "/path/to/token".to_string(),
        target: "/run/secrets/token".to_string(),
        when_profiles: vec!["nonexistent".to_string()],
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("unknown profile"));
}

#[test]
fn rejects_dispatch_rule_referencing_unknown_profile() {
    let mut config = base_config();
    config.dispatch.insert(
        "unknown".to_string(),
        DispatchRule {
            patterns: vec!["cargo *".to_string()],
            profile: "nonexistent".to_string(),
        },
    );

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("unknown profile"));
}

#[test]
fn rejects_dispatch_rule_with_no_patterns() {
    let mut config = base_config();
    config.dispatch.insert(
        "empty".to_string(),
        DispatchRule {
            patterns: Vec::new(),
            profile: "default".to_string(),
        },
    );

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("pattern"));
}

#[test]
fn accepts_image_with_only_reference() {
    let mut config = base_config();
    config.image = Some(ImageConfig {
        reference: Some("python:3.13-slim".to_string()),
        build: None,
        preset: None,
        digest: None,
        verify_signature: None,
        pull_policy: None,
        tag: None,
    });

    validate_config(&config).expect("valid image should pass");
}

#[test]
fn accepts_image_with_only_build() {
    let mut config = base_config();
    config.image = Some(ImageConfig {
        reference: None,
        build: Some(PathBuf::from("Dockerfile")),
        preset: None,
        digest: None,
        verify_signature: None,
        pull_policy: None,
        tag: None,
    });

    validate_config(&config).expect("valid image should pass");
}

#[test]
fn accepts_image_with_only_preset() {
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

    validate_config(&config).expect("valid image should pass");
}

#[test]
fn rejects_image_with_multiple_sources() {
    let mut config = base_config();
    config.image = Some(ImageConfig {
        reference: Some("python:3.13-slim".to_string()),
        build: Some(PathBuf::from("Dockerfile")),
        preset: None,
        digest: None,
        verify_signature: None,
        pull_policy: None,
        tag: None,
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("mutually exclusive"));
}

#[test]
fn rejects_image_with_no_sources() {
    let mut config = base_config();
    config.image = Some(ImageConfig {
        reference: None,
        build: None,
        preset: None,
        digest: None,
        verify_signature: None,
        pull_policy: None,
        tag: None,
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(error.to_string().contains("ref"));
}

#[test]
fn validates_multiple_errors_at_once() {
    let mut config = base_config();
    config.version = 99;
    config.runtime = None;
    config.workspace = None;
    config.image = None;

    let error = validate_config(&config).expect_err("validation should fail");
    let error_str = error.to_string();
    assert!(error_str.contains("unsupported config version"));
    assert!(error_str.contains("runtime"));
    assert!(error_str.contains("workspace"));
    assert!(error_str.contains("image"));
}

#[test]
fn rejects_unknown_capabilities_keyword() {
    let mut config = base_config();
    config.profiles.get_mut("default").unwrap().capabilities =
        Some(CapabilitiesSpec::Keyword("all".to_string()));

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(
        error.to_string().contains("unknown capabilities keyword"),
        "expected unknown capabilities keyword error, got: {}",
        error
    );
}

#[test]
fn accepts_drop_all_capabilities_keyword() {
    let mut config = base_config();
    config.profiles.get_mut("default").unwrap().capabilities =
        Some(CapabilitiesSpec::Keyword("drop-all".to_string()));

    validate_config(&config).expect("drop-all should be valid");
}

#[test]
fn rejects_map_user_true_when_rootless_is_false() {
    let mut config = base_config();
    config.runtime = Some(RuntimeConfig {
        backend: Some(BackendKind::Podman),
        rootless: Some(false),
        strict_security: None,
        reuse_container: Some(false),
        container_name: None,
        pull_policy: None,
        require_pinned_image: None,
    });
    config.identity = Some(IdentityConfig {
        uid: None,
        gid: None,
        map_user: Some(true),
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(
        error.to_string().contains("conflicts with"),
        "expected conflict error, got: {}",
        error
    );
}

#[test]
fn rejects_absolute_writable_path() {
    let mut config = base_config();
    config.workspace = Some(WorkspaceConfig {
        root: Some(PathBuf::from(".")),
        mount: Some("/workspace".to_string()),
        writable: Some(false),
        writable_paths: vec!["/absolute/path".to_string()],
        exclude_paths: Vec::new(),
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(
        error.to_string().contains("relative path"),
        "expected relative path error, got: {}",
        error
    );
}

#[test]
fn rejects_writable_path_with_traversal() {
    let mut config = base_config();
    config.workspace = Some(WorkspaceConfig {
        root: Some(PathBuf::from(".")),
        mount: Some("/workspace".to_string()),
        writable: Some(false),
        writable_paths: vec!["../escape".to_string()],
        exclude_paths: Vec::new(),
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(
        error.to_string().contains(".."),
        "expected traversal error, got: {}",
        error
    );
}

#[test]
fn rejects_empty_writable_path_entry() {
    let mut config = base_config();
    config.workspace = Some(WorkspaceConfig {
        root: Some(PathBuf::from(".")),
        mount: Some("/workspace".to_string()),
        writable: Some(false),
        writable_paths: vec!["  ".to_string()],
        exclude_paths: Vec::new(),
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(
        error.to_string().contains("must not be empty"),
        "expected empty entry error, got: {}",
        error
    );
}

#[test]
fn accepts_valid_writable_paths() {
    let mut config = base_config();
    config.workspace = Some(WorkspaceConfig {
        root: Some(PathBuf::from(".")),
        mount: Some("/workspace".to_string()),
        writable: Some(false),
        writable_paths: vec!["node_modules".to_string(), "dist".to_string()],
        exclude_paths: Vec::new(),
    });

    validate_config(&config).expect("valid writable_paths should be accepted");
}

#[test]
fn rejects_absolute_exclude_path() {
    let mut config = base_config();
    config.workspace = Some(WorkspaceConfig {
        root: Some(PathBuf::from(".")),
        mount: Some("/workspace".to_string()),
        writable: Some(true),
        writable_paths: Vec::new(),
        exclude_paths: vec!["/etc/passwd".to_string()],
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(
        error.to_string().contains("relative pattern"),
        "expected relative pattern error, got: {}",
        error
    );
}

#[test]
fn rejects_empty_exclude_path_entry() {
    let mut config = base_config();
    config.workspace = Some(WorkspaceConfig {
        root: Some(PathBuf::from(".")),
        mount: Some("/workspace".to_string()),
        writable: Some(true),
        writable_paths: Vec::new(),
        exclude_paths: vec!["".to_string()],
    });

    let error = validate_config(&config).expect_err("validation should fail");
    assert!(
        error.to_string().contains("must not be empty"),
        "expected empty entry error, got: {}",
        error
    );
}

#[test]
fn accepts_valid_exclude_paths() {
    let mut config = base_config();
    config.workspace = Some(WorkspaceConfig {
        root: Some(PathBuf::from(".")),
        mount: Some("/workspace".to_string()),
        writable: Some(true),
        writable_paths: Vec::new(),
        exclude_paths: vec![
            ".env".to_string(),
            "*.pem".to_string(),
            "**/*.key".to_string(),
            ".env.local".to_string(),
        ],
    });

    validate_config(&config).expect("valid exclude_paths should be accepted");
}
