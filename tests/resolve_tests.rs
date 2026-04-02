use indexmap::IndexMap;
use std::collections::BTreeMap;
use std::path::PathBuf;

use sbox::cli::{Cli, CliBackendKind, CliExecutionMode, Commands, ExecCommand, PlanCommand};
use sbox::config::load::LoadedConfig;
use sbox::config::model::{
    BackendKind, Config, DispatchRule, EnvironmentConfig, ExecutionMode, ImageConfig,
    ProfileConfig, RuntimeConfig, WorkspaceConfig,
};
use sbox::resolve::{
    ProfileSource, ResolutionTarget, ResolvedImageSource, ResolvedUser, resolve_execution_plan,
};

fn minimal_config() -> Config {
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
    profiles.insert(
        "build".to_string(),
        ProfileConfig {
            mode: ExecutionMode::Sandbox,
            image: None,
            network: Some("off".to_string()),
            writable: Some(true),
            require_pinned_image: None,
            require_lockfile: None,
            script_policy: None,
            ports: vec!["8080:80".to_string()],
            audit_hooks: Vec::new(),
            capabilities: None,
            no_new_privileges: Some(true),
            read_only_rootfs: None,
            reuse_container: None,
            shell: None,
        },
    );
    profiles.insert(
        "host".to_string(),
        ProfileConfig {
            mode: ExecutionMode::Host,
            image: None,
            network: Some("on".to_string()),
            writable: Some(true),
            require_pinned_image: None,
            require_lockfile: None,
            script_policy: None,
            ports: Vec::new(),
            audit_hooks: Vec::new(),
            capabilities: None,
            no_new_privileges: None,
            read_only_rootfs: None,
            reuse_container: None,
            shell: None,
        },
    );

    let mut dispatch = IndexMap::new();
    dispatch.insert(
        "install-dispatch".to_string(),
        DispatchRule {
            patterns: vec![
                "npm install".to_string(),
                "npm ci".to_string(),
                "uv sync".to_string(),
            ],
            profile: "install".to_string(),
        },
    );
    dispatch.insert(
        "build-dispatch".to_string(),
        DispatchRule {
            patterns: vec!["cargo build*".to_string(), "cargo test*".to_string()],
            profile: "build".to_string(),
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
        }),
        workspace: Some(WorkspaceConfig {
            root: Some(PathBuf::from("/workspace/project")),
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
        environment: Some(EnvironmentConfig {
            pass_through: vec!["TERM".to_string(), "HOME".to_string()],
            set: BTreeMap::from([("RUST_BACKTRACE".to_string(), "1".to_string())]),
            deny: Vec::new(),
        }),
        mounts: Vec::new(),
        caches: Vec::new(),
        secrets: Vec::new(),
        profiles,
        dispatch,
    }
}

fn make_loaded_config() -> LoadedConfig {
    LoadedConfig {
        invocation_dir: PathBuf::from("/workspace/project"),
        workspace_root: PathBuf::from("/workspace/project"),
        config_path: PathBuf::from("/workspace/project/sbox.yaml"),
        config: minimal_config(),
    }
}

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
            command: vec!["echo".into(), "hello".into()],
        }),
    }
}

fn resolve(
    cli: &Cli,
    command: &[String],
    target: ResolutionTarget<'_>,
) -> sbox::resolve::ExecutionPlan {
    resolve_execution_plan(cli, &make_loaded_config(), target, command)
        .expect("resolution should succeed")
}

#[test]
fn selects_default_profile_when_no_dispatch_matches() {
    let cli = base_cli();
    let plan = resolve(
        &cli,
        &["echo".into(), "hello".into()],
        ResolutionTarget::Plan,
    );

    assert_eq!(plan.profile_name, "default");
    assert!(matches!(plan.profile_source, ProfileSource::DefaultProfile));
}

#[test]
fn dispatch_overrides_default_profile() {
    let cli = base_cli();
    let plan = resolve(
        &cli,
        &["npm".into(), "install".into()],
        ResolutionTarget::Plan,
    );

    assert_eq!(plan.profile_name, "install");
    match plan.profile_source {
        ProfileSource::Dispatch { rule_name, pattern } => {
            assert_eq!(rule_name, "install-dispatch");
            assert_eq!(pattern, "npm install");
        }
        other => panic!("expected dispatch source, got {other:?}"),
    }
}

#[test]
fn dispatch_supports_wildcard_patterns() {
    let cli = base_cli();
    let plan = resolve(
        &cli,
        &["cargo".into(), "build".into(), "--release".into()],
        ResolutionTarget::Plan,
    );

    assert_eq!(plan.profile_name, "build");
    match plan.profile_source {
        ProfileSource::Dispatch { pattern, .. } => {
            assert_eq!(pattern, "cargo build*");
        }
        other => panic!("expected dispatch source, got {other:?}"),
    }
}

#[test]
fn cli_profile_flag_overrides_dispatch() {
    let cli = Cli {
        profile: Some("host".to_string()),
        ..base_cli()
    };
    let plan = resolve(
        &cli,
        &["npm".into(), "install".into()],
        ResolutionTarget::Plan,
    );

    assert_eq!(plan.profile_name, "host");
    assert!(matches!(plan.profile_source, ProfileSource::CliOverride));
}

#[test]
fn exec_subcommand_selects_profile() {
    let cli = Cli {
        command: Commands::Exec(ExecCommand {
            profile: "install".to_string(),
            command: vec!["pip".into(), "install".into(), "requests".into()],
        }),
        ..base_cli()
    };

    let plan = resolve(
        &cli,
        &["pip".into(), "install".into(), "requests".into()],
        ResolutionTarget::Exec { profile: "install" },
    );

    assert_eq!(plan.profile_name, "install");
    assert!(matches!(plan.profile_source, ProfileSource::ExecSubcommand));
}

#[test]
fn cli_backend_overrides_config() {
    let cli = Cli {
        backend: Some(CliBackendKind::Docker),
        ..base_cli()
    };

    let plan = resolve(&cli, &["echo".into()], ResolutionTarget::Plan);

    assert!(matches!(plan.backend, BackendKind::Docker));
}

#[test]
fn cli_image_overrides_config() {
    let cli = Cli {
        image: Some("rust:1.80".to_string()),
        ..base_cli()
    };

    let plan = resolve(&cli, &["echo".into()], ResolutionTarget::Plan);

    match &plan.image.source {
        ResolvedImageSource::Reference(image) => {
            assert!(image.contains("rust:1.80"));
            assert!(plan.image.description.contains("cli override"));
        }
        other => panic!("expected reference, got {other:?}"),
    }
}

#[test]
fn profile_image_overrides_global_image() {
    let cli = base_cli();
    let mut config = minimal_config();
    config.profiles.get_mut("build").unwrap().image = Some(ImageConfig {
        reference: Some("rust:1-bookworm".to_string()),
        build: None,
        preset: None,
        digest: None,
        verify_signature: None,
        pull_policy: None,
        tag: None,
    });

    let loaded = LoadedConfig {
        invocation_dir: PathBuf::from("/workspace/project"),
        workspace_root: PathBuf::from("/workspace/project"),
        config_path: PathBuf::from("/workspace/project/sbox.yaml"),
        config,
    };
    let plan = resolve_execution_plan(
        &cli,
        &loaded,
        ResolutionTarget::Plan,
        &["cargo".into(), "build".into()],
    )
    .expect("resolution should succeed");

    assert!(matches!(
        plan.image.source,
        ResolvedImageSource::Reference(ref image) if image == "rust:1-bookworm"
    ));
    assert!(plan.image.description.contains("profile override"));
}

#[test]
fn cli_image_override_beats_profile_image_override() {
    let cli = Cli {
        image: Some("node:22-bookworm-slim".to_string()),
        ..base_cli()
    };
    let mut config = minimal_config();
    config.profiles.get_mut("build").unwrap().image = Some(ImageConfig {
        reference: Some("rust:1-bookworm".to_string()),
        build: None,
        preset: None,
        digest: None,
        verify_signature: None,
        pull_policy: None,
        tag: None,
    });

    let loaded = LoadedConfig {
        invocation_dir: PathBuf::from("/workspace/project"),
        workspace_root: PathBuf::from("/workspace/project"),
        config_path: PathBuf::from("/workspace/project/sbox.yaml"),
        config,
    };
    let plan = resolve_execution_plan(
        &cli,
        &loaded,
        ResolutionTarget::Plan,
        &["cargo".into(), "build".into()],
    )
    .expect("resolution should succeed");

    assert!(matches!(
        plan.image.source,
        ResolvedImageSource::Reference(ref image) if image == "node:22-bookworm-slim"
    ));
    assert!(plan.image.description.contains("cli override"));
}

#[test]
fn cli_mode_overrides_profile() {
    let cli = Cli {
        mode: Some(CliExecutionMode::Host),
        ..base_cli()
    };

    let plan = resolve(&cli, &["echo".into()], ResolutionTarget::Plan);

    assert!(matches!(plan.mode, ExecutionMode::Host));
}

#[test]
fn sandbox_mode_preserves_ports() {
    let cli = base_cli();
    let plan = resolve(
        &cli,
        &["cargo".into(), "build".into()],
        ResolutionTarget::Plan,
    );

    assert!(matches!(plan.mode, ExecutionMode::Sandbox));
    assert_eq!(plan.policy.ports, vec!["8080:80"]);
}

#[test]
fn host_mode_strips_ports() {
    let cli = Cli {
        mode: Some(CliExecutionMode::Host),
        ..base_cli()
    };

    let plan = resolve(
        &cli,
        &["cargo".into(), "build".into()],
        ResolutionTarget::Plan,
    );

    assert!(matches!(plan.mode, ExecutionMode::Host));
    assert!(plan.policy.ports.is_empty());
}

#[test]
fn workspace_cwd_falls_back_to_root_when_outside() {
    let loaded = LoadedConfig {
        invocation_dir: PathBuf::from("/home/user/other"),
        workspace_root: PathBuf::from("/workspace/project"),
        config_path: PathBuf::from("/workspace/project/sbox.yaml"),
        config: minimal_config(),
    };

    let cli = base_cli();
    let plan = resolve_execution_plan(&cli, &loaded, ResolutionTarget::Plan, &["echo".into()])
        .expect("resolution should succeed");

    assert_eq!(
        plan.workspace.effective_host_dir,
        PathBuf::from("/workspace/project")
    );
    assert_eq!(plan.workspace.sandbox_cwd, "/workspace");
}

#[test]
fn network_off_by_default_in_sandbox() {
    let cli = base_cli();
    let plan = resolve(&cli, &["echo".into()], ResolutionTarget::Plan);

    assert_eq!(plan.policy.network, "off");
}

#[test]
fn network_on_in_install_profile() {
    let cli = base_cli();
    let plan = resolve(
        &cli,
        &["npm".into(), "install".into()],
        ResolutionTarget::Plan,
    );

    assert_eq!(plan.policy.network, "on");
}

#[test]
fn no_new_privileges_enabled_by_default() {
    let cli = base_cli();
    let plan = resolve(&cli, &["echo".into()], ResolutionTarget::Plan);

    assert!(plan.policy.no_new_privileges);
}

#[test]
fn install_style_command_detected() {
    let cli = base_cli();

    let plan = resolve(
        &cli,
        &["npm".into(), "install".into()],
        ResolutionTarget::Plan,
    );
    assert!(plan.audit.install_style);

    let plan = resolve(&cli, &["npm".into(), "ci".into()], ResolutionTarget::Plan);
    assert!(plan.audit.install_style);

    let plan = resolve(&cli, &["uv".into(), "sync".into()], ResolutionTarget::Plan);
    assert!(plan.audit.install_style);

    let plan = resolve(
        &cli,
        &["pip".into(), "install".into(), "requests".into()],
        ResolutionTarget::Plan,
    );
    assert!(plan.audit.install_style);

    let plan = resolve(
        &cli,
        &["cargo".into(), "install".into(), "ripgrep".into()],
        ResolutionTarget::Plan,
    );
    assert!(plan.audit.install_style);
}

#[test]
fn non_install_command_not_marked_as_install_style() {
    let cli = base_cli();

    let plan = resolve(
        &cli,
        &["echo".into(), "hello".into()],
        ResolutionTarget::Plan,
    );
    assert!(!plan.audit.install_style);

    let plan = resolve(
        &cli,
        &["cargo".into(), "build".into()],
        ResolutionTarget::Plan,
    );
    assert!(!plan.audit.install_style);
}

#[test]
fn preset_python_resolves_to_correct_image() {
    let mut config = minimal_config();
    config.image = Some(ImageConfig {
        reference: None,
        build: None,
        preset: Some("python".to_string()),
        digest: None,
        verify_signature: None,
        pull_policy: None,
        tag: None,
    });

    let loaded = LoadedConfig {
        invocation_dir: PathBuf::from("/workspace/project"),
        workspace_root: PathBuf::from("/workspace/project"),
        config_path: PathBuf::from("/workspace/project/sbox.yaml"),
        config,
    };

    let cli = base_cli();
    let plan = resolve_execution_plan(&cli, &loaded, ResolutionTarget::Plan, &["echo".into()])
        .expect("resolution should succeed");

    match &plan.image.source {
        ResolvedImageSource::Reference(image) => {
            assert!(image.contains("python"));
        }
        other => panic!("expected reference, got {other:?}"),
    }
}

#[test]
fn preset_node_resolves_to_correct_image() {
    let mut config = minimal_config();
    config.image = Some(ImageConfig {
        reference: None,
        build: None,
        preset: Some("node".to_string()),
        digest: None,
        verify_signature: None,
        pull_policy: None,
        tag: None,
    });

    let loaded = LoadedConfig {
        invocation_dir: PathBuf::from("/workspace/project"),
        workspace_root: PathBuf::from("/workspace/project"),
        config_path: PathBuf::from("/workspace/project/sbox.yaml"),
        config,
    };

    let cli = base_cli();
    let plan = resolve_execution_plan(&cli, &loaded, ResolutionTarget::Plan, &["echo".into()])
        .expect("resolution should succeed");

    match &plan.image.source {
        ResolvedImageSource::Reference(image) => {
            assert!(image.contains("node"));
        }
        other => panic!("expected reference, got {other:?}"),
    }
}

#[test]
fn preset_rust_resolves_to_correct_image() {
    let mut config = minimal_config();
    config.image = Some(ImageConfig {
        reference: None,
        build: None,
        preset: Some("rust".to_string()),
        digest: None,
        verify_signature: None,
        pull_policy: None,
        tag: None,
    });

    let loaded = LoadedConfig {
        invocation_dir: PathBuf::from("/workspace/project"),
        workspace_root: PathBuf::from("/workspace/project"),
        config_path: PathBuf::from("/workspace/project/sbox.yaml"),
        config,
    };

    let cli = base_cli();
    let plan = resolve_execution_plan(&cli, &loaded, ResolutionTarget::Plan, &["echo".into()])
        .expect("resolution should succeed");

    match &plan.image.source {
        ResolvedImageSource::Reference(image) => {
            assert!(image.contains("rust"));
        }
        other => panic!("expected reference, got {other:?}"),
    }
}

#[test]
fn unknown_preset_returns_error() {
    let mut config = minimal_config();
    config.image = Some(ImageConfig {
        reference: None,
        build: None,
        preset: Some("nonexistent".to_string()),
        digest: None,
        verify_signature: None,
        pull_policy: None,
        tag: None,
    });

    let loaded = LoadedConfig {
        invocation_dir: PathBuf::from("/workspace/project"),
        workspace_root: PathBuf::from("/workspace/project"),
        config_path: PathBuf::from("/workspace/project/sbox.yaml"),
        config,
    };

    let cli = base_cli();
    let result = resolve_execution_plan(&cli, &loaded, ResolutionTarget::Plan, &["echo".into()]);

    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(error.to_string().contains("unknown preset"));
}

#[test]
fn explicit_uid_gid_resolves_user() {
    let mut config = minimal_config();
    config.identity = Some(sbox::config::model::IdentityConfig {
        map_user: Some(false),
        uid: Some(1000),
        gid: Some(1000),
    });

    let loaded = LoadedConfig {
        invocation_dir: PathBuf::from("/workspace/project"),
        workspace_root: PathBuf::from("/workspace/project"),
        config_path: PathBuf::from("/workspace/project/sbox.yaml"),
        config,
    };

    let cli = base_cli();
    let plan = resolve_execution_plan(&cli, &loaded, ResolutionTarget::Plan, &["echo".into()])
        .expect("resolution should succeed");

    assert!(matches!(
        plan.user,
        ResolvedUser::Explicit {
            uid: 1000,
            gid: 1000
        }
    ));
}

#[test]
fn default_identity_keeps_user_id() {
    let cli = base_cli();
    let plan = resolve(&cli, &["echo".into()], ResolutionTarget::Plan);

    assert!(matches!(plan.user, ResolvedUser::KeepId));
}

#[test]
fn command_string_preserved_in_plan() {
    let cli = base_cli();
    let plan = resolve(
        &cli,
        &["cargo".into(), "build".into(), "--release".into()],
        ResolutionTarget::Plan,
    );

    assert_eq!(plan.command_string, "cargo build --release");
    assert_eq!(plan.command, vec!["cargo", "build", "--release"]);
}

#[test]
fn reusable_session_name_includes_workspace_hash() {
    let mut config = minimal_config();
    config.runtime.as_mut().unwrap().reuse_container = Some(true);

    let loaded = LoadedConfig {
        invocation_dir: PathBuf::from("/workspace/project"),
        workspace_root: PathBuf::from("/workspace/project"),
        config_path: PathBuf::from("/workspace/project/sbox.yaml"),
        config,
    };

    let cli = base_cli();
    let plan = resolve_execution_plan(&cli, &loaded, ResolutionTarget::Plan, &["echo".into()])
        .expect("resolution should succeed");

    assert!(plan.policy.reuse_container);
    let session_name = plan
        .policy
        .reusable_session_name
        .expect("session name should exist");
    assert!(session_name.starts_with("sbox-"));
    assert!(session_name.contains("default"));
}
