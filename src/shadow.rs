use std::fs;
use std::path::{Path, PathBuf};

use crate::config::BackendKind;
use crate::error::SboxError;
use crate::resolve::{ExecutionPlan, ResolvedImage, ResolvedImageSource, ImageTrust, ResolvedWorkspace, ResolvedPolicy, ResolvedEnvironment, ResolvedUser, CwdMapping, ExecutionAudit, LockfileAudit};
use crate::config::model::{ExecutionMode, NetworkPolicy};

#[derive(Debug, Clone)]
pub enum InfraKind {
    Compose { file: PathBuf, service: Option<String> },
    Dockerfile(PathBuf),
    None,
}

pub fn detect_infrastructure(workspace: &Path, backend: BackendKind) -> InfraKind {
    let compose_files = match backend {
        BackendKind::Podman => vec!["podman-compose.yml", "podman-compose.yaml", "docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml"],
        BackendKind::Docker => vec!["docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml"],
    };

    for file in compose_files {
        let path = workspace.join(file);
        if path.exists() {
            return InfraKind::Compose { file: path, service: None };
        }
    }

    let dockerfile = workspace.join("Dockerfile");
    if dockerfile.exists() {
        return InfraKind::Dockerfile(dockerfile);
    }

    InfraKind::None
}

pub fn resolve_shadow_plan(
    infra: InfraKind,
    command: Vec<String>,
    workspace_root: &Path,
    invocation_dir: &Path,
    backend: BackendKind,
) -> Result<ExecutionPlan, SboxError> {
    match infra {
        InfraKind::Compose { file, service } => {
            resolve_compose_shadow(file, service, command, workspace_root, invocation_dir, backend)
        }
        InfraKind::Dockerfile(path) => {
            resolve_dockerfile_shadow(path, command, workspace_root, invocation_dir, backend)
        }
        InfraKind::None => {
            resolve_default_shadow(command, workspace_root, invocation_dir, backend)
        }
    }
}

fn resolve_compose_shadow(
    file: PathBuf,
    service: Option<String>,
    command: Vec<String>,
    workspace_root: &Path,
    invocation_dir: &Path,
    backend: BackendKind,
) -> Result<ExecutionPlan, SboxError> {
    // For now, let's implement a simplified version that extracts the image from the compose file.
    // In a full implementation, we would generate a shadow compose file.
    
    let content = fs::read_to_string(&file).map_err(|e| SboxError::InfrastructureDetectionFailed { 
        reason: format!("failed to read compose file: {}", e) 
    })?;
    
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| SboxError::ShadowComposeGeneration { source: e })?;
    
    let services = yaml.get("services").and_then(|s| s.as_mapping()).ok_or_else(|| {
        SboxError::InfrastructureDetectionFailed { reason: "invalid compose file: no services found".to_string() }
    })?;
    
    let target_service = service.or_else(|| {
        let candidates = ["app", "web", "api", "backend", "server"];
        for c in candidates {
            if services.contains_key(&serde_yaml::Value::String(c.to_string())) {
                return Some(c.to_string());
            }
        }
        // Fallback to first service
        services.keys().next().and_then(|k| k.as_str()).map(|s| s.to_string())
    }).ok_or_else(|| SboxError::InfrastructureDetectionFailed { reason: "no services found in compose file".to_string() })?;

    let service_val = services.get(&serde_yaml::Value::String(target_service.clone())).unwrap();
    let image_ref = service_val.get("image").and_then(|i| i.as_str()).map(|s| s.to_string())
        .unwrap_or_else(|| "ubuntu:24.04".to_string()); // Fallback if no image specified (e.g. build only)

    let mut plan = base_shadow_plan(command, workspace_root, invocation_dir, backend);
    plan.image = ResolvedImage {
        description: format!("Inherited from compose service '{}'", target_service),
        source: ResolvedImageSource::Reference(image_ref),
        trust: ImageTrust::MutableReference,
        verify_signature: false,
    };
    plan.profile_name = format!("shadow-compose-{}", target_service);
    
    // TODO: In Case A, we should also handle sidecars and shadow compose file.
    // For this MVP, we focus on running the command in the right image with sbox security.

    Ok(plan)
}

fn resolve_dockerfile_shadow(
    path: PathBuf,
    command: Vec<String>,
    workspace_root: &Path,
    invocation_dir: &Path,
    backend: BackendKind,
) -> Result<ExecutionPlan, SboxError> {
    let mut plan = base_shadow_plan(command, workspace_root, invocation_dir, backend);
    
    let tag = format!("sbox-shadow-{}", std::process::id());
    
    plan.image = ResolvedImage {
        description: "Built from Dockerfile".to_string(),
        source: ResolvedImageSource::Build { 
            recipe_path: path,
            tag: tag.clone(),
        },
        trust: ImageTrust::LocalBuild,
        verify_signature: false,
    };
    plan.profile_name = "shadow-dockerfile".to_string();

    Ok(plan)
}

fn resolve_default_shadow(
    command: Vec<String>,
    workspace_root: &Path,
    invocation_dir: &Path,
    backend: BackendKind,
) -> Result<ExecutionPlan, SboxError> {
    let mut plan = base_shadow_plan(command, workspace_root, invocation_dir, backend);
    plan.image = ResolvedImage {
        description: "Default secure image".to_string(),
        source: ResolvedImageSource::Reference("ubuntu:24.04".to_string()),
        trust: ImageTrust::MutableReference,
        verify_signature: false,
    };
    plan.profile_name = "shadow-default".to_string();
    // In Case C, default network to off
    plan.policy.network = "off".to_string();

    Ok(plan)
}

fn base_shadow_plan(
    command: Vec<String>,
    workspace_root: &Path,
    invocation_dir: &Path,
    backend: BackendKind,
) -> ExecutionPlan {
    let command_string = command.join(" ");
    
    ExecutionPlan {
        command,
        command_string,
        backend,
        image: ResolvedImage {
            description: "".to_string(),
            source: ResolvedImageSource::Reference("".to_string()),
            trust: ImageTrust::MutableReference,
            verify_signature: false,
        },
        profile_name: "shadow".to_string(),
        profile_source: crate::resolve::ProfileSource::Shadow,
        mode: ExecutionMode::Sandbox,
        mode_source: crate::resolve::ModeSource::Default,
        workspace: ResolvedWorkspace {
            root: workspace_root.to_path_buf(),
            invocation_dir: invocation_dir.to_path_buf(),
            effective_host_dir: workspace_root.to_path_buf(),
            mount: "/src".to_string(),
            sandbox_cwd: "/src".to_string(), // Simplified
            cwd_mapping: CwdMapping::WorkspaceRootFallback,
        },
        policy: ResolvedPolicy {
            network: "on".to_string(),
            network_policy: NetworkPolicy::Dns,
            writable: false,
            ports: Vec::new(),
            no_new_privileges: true,
            read_only_rootfs: true,
            reuse_container: false,
            reusable_session_name: None,
            cap_drop: vec!["ALL".to_string()],
            cap_add: Vec::new(),
            pull_policy: None,
            network_allow: Vec::new(),
            network_allow_patterns: Vec::new(),
        },
        environment: ResolvedEnvironment {
            variables: Vec::new(),
            denied: Vec::new(),
        },
        mounts: Vec::new(),
        caches: Vec::new(),
        secrets: Vec::new(),
        user: ResolvedUser::KeepId,
        rootless: true, // Assuming rootless for now
        audit: ExecutionAudit {
            install_style: false,
            trusted_image_required: false,
            sensitive_pass_through_vars: Vec::new(),
            lockfile: LockfileAudit {
                applicable: false,
                required: false,
                present: false,
                expected_files: Vec::new(),
            },
            pre_run: Vec::new(),
        },
        compose: None,
    }
}
