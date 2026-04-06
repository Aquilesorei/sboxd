use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use crate::cli::{CleanCommand, Cli};
use crate::config::{LoadOptions, load_config};
use crate::error::SboxError;

pub fn execute(cli: &Cli, command: &CleanCommand) -> Result<ExitCode, SboxError> {
    if command.global_scope {
        return execute_global();
    }

    let scope = CleanScope::from_command(command);
    let loaded = load_config(&LoadOptions {
        workspace: cli.workspace.clone(),
        config: cli.config.clone(),
    })?;

    let mut removed = Vec::new();

    if scope.sessions {
        let session_names = reusable_session_names(&loaded.config, &loaded.workspace_root);
        if session_names.is_empty() {
            println!("sessions: no reusable sessions configured for this workspace");
        } else {
            for name in session_names {
                remove_podman_container(&name)?;
                removed.push(format!("session:{name}"));
            }
        }
    }

    if scope.images {
        if let Some(tag) = derived_image_tag(&loaded.config, &loaded.workspace_root) {
            remove_podman_image(&tag)?;
            removed.push(format!("image:{tag}"));
        } else {
            println!("images: no workspace-derived image configured");
        }
    }

    if scope.caches {
        let names = cache_volume_names(&loaded.config.caches, &loaded.workspace_root);
        if names.is_empty() {
            println!("caches: no workspace caches configured");
        } else {
            for name in names {
                remove_podman_volume(&name)?;
                removed.push(format!("cache:{name}"));
            }
        }
    }

    if removed.is_empty() {
        println!("clean: nothing removed");
    } else {
        println!("clean: removed {}", removed.join(", "));
    }

    Ok(ExitCode::SUCCESS)
}

/// Remove all sbox-managed containers and volumes across every project on this host.
/// Identifies resources by the `sbox-` name prefix (the same convention used when creating them).
fn execute_global() -> Result<ExitCode, SboxError> {
    let containers = list_podman_names(&["ps", "-a", "--format", "{{.Names}}"], "sbox-")?;
    let volumes = list_podman_names(&["volume", "ls", "--format", "{{.Name}}"], "sbox-")?;
    let images = list_podman_names(
        &["images", "--format", "{{.Repository}}:{{.Tag}}"],
        "sbox-build-",
    )?;

    let mut removed = Vec::new();

    for name in &containers {
        remove_podman_container(name)?;
        removed.push(format!("container:{name}"));
    }
    for name in &volumes {
        remove_podman_volume(name)?;
        removed.push(format!("volume:{name}"));
    }
    for tag in &images {
        remove_podman_image(tag)?;
        removed.push(format!("image:{tag}"));
    }

    if removed.is_empty() {
        println!("clean --global: no sbox-managed resources found");
    } else {
        println!("clean --global: removed {}", removed.join(", "));
    }

    Ok(ExitCode::SUCCESS)
}

fn list_podman_names(args: &[&str], prefix: &str) -> Result<Vec<String>, SboxError> {
    let output = Command::new("podman")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| name.starts_with(prefix))
        .map(String::from)
        .collect())
}

#[derive(Debug, Clone, Copy)]
struct CleanScope {
    sessions: bool,
    images: bool,
    caches: bool,
}

impl CleanScope {
    fn from_command(command: &CleanCommand) -> Self {
        if command.all {
            return Self {
                sessions: true,
                images: true,
                caches: true,
            };
        }

        if !command.sessions && !command.images && !command.caches {
            return Self {
                sessions: true,
                images: false,
                caches: false,
            };
        }

        Self {
            sessions: command.sessions,
            images: command.images,
            caches: command.caches,
        }
    }
}

fn derived_image_tag(
    config: &crate::config::model::Config,
    workspace_root: &Path,
) -> Option<String> {
    let image = config.image.as_ref()?;
    let build = image.build.as_ref()?;
    if let Some(tag) = &image.tag {
        return Some(tag.clone());
    }

    let recipe_path = if build.is_absolute() {
        build.clone()
    } else {
        workspace_root.join(build)
    };

    Some(format!(
        "sbox-build-{}",
        stable_hash(&recipe_path.display().to_string())
    ))
}

fn cache_volume_names(
    caches: &[crate::config::model::CacheConfig],
    workspace_root: &Path,
) -> Vec<String> {
    caches
        .iter()
        .filter(|cache| cache.source.is_none())
        .map(|cache| {
            format!(
                "sbox-cache-{}-{}",
                stable_hash(&workspace_root.display().to_string()),
                sanitize_volume_name(&cache.name)
            )
        })
        .collect()
}

fn reusable_session_names(
    config: &crate::config::model::Config,
    workspace_root: &Path,
) -> Vec<String> {
    let runtime_reuse = config
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.reuse_container)
        .unwrap_or(false);
    let template = config
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.container_name.as_ref());

    config
        .profiles
        .iter()
        .filter(|(_, profile)| profile.reuse_container.unwrap_or(runtime_reuse))
        .map(|(profile_name, _)| reusable_session_name(template, workspace_root, profile_name))
        .collect()
}

fn remove_podman_image(tag: &str) -> Result<(), SboxError> {
    let status = Command::new("podman")
        .args(["image", "rm", "-f", tag])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    if status.success() || status.code() == Some(1) {
        Ok(())
    } else {
        Err(SboxError::BackendCommandFailed {
            backend: "podman".to_string(),
            command: format!("podman image rm -f {tag}"),
            status: status.code().unwrap_or(1),
        })
    }
}

fn remove_podman_volume(name: &str) -> Result<(), SboxError> {
    let status = Command::new("podman")
        .args(["volume", "rm", "-f", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    if status.success() || status.code() == Some(1) {
        Ok(())
    } else {
        Err(SboxError::BackendCommandFailed {
            backend: "podman".to_string(),
            command: format!("podman volume rm -f {name}"),
            status: status.code().unwrap_or(1),
        })
    }
}

fn remove_podman_container(name: &str) -> Result<(), SboxError> {
    let status = Command::new("podman")
        .args(["rm", "-f", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    if status.success() || status.code() == Some(1) {
        Ok(())
    } else {
        Err(SboxError::BackendCommandFailed {
            backend: "podman".to_string(),
            command: format!("podman rm -f {name}"),
            status: status.code().unwrap_or(1),
        })
    }
}

fn sanitize_volume_name(name: &str) -> String {
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

fn stable_hash(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn reusable_session_name(
    template: Option<&String>,
    workspace_root: &Path,
    profile_name: &str,
) -> String {
    let workspace_hash = stable_hash(&workspace_root.display().to_string());
    let base = template
        .map(|template| {
            template
                .replace("{profile}", profile_name)
                .replace("{workspace_hash}", &workspace_hash)
        })
        .unwrap_or_else(|| format!("sbox-{workspace_hash}-{profile_name}"));

    sanitize_volume_name(&base)
}

#[cfg(test)]
mod tests {
    use super::{CleanScope, cache_volume_names, reusable_session_names};
    use crate::cli::CleanCommand;
    use crate::config::model::{
        BackendKind, CacheConfig, Config, ExecutionMode, ProfileConfig, RuntimeConfig,
    };
    use indexmap::IndexMap;
    use std::path::Path;

    #[test]
    fn defaults_to_sessions_only() {
        let scope = CleanScope::from_command(&CleanCommand::default());
        assert!(scope.sessions);
        assert!(!scope.images);
        assert!(!scope.caches);
    }

    #[test]
    fn all_flag_enables_every_cleanup_target() {
        let scope = CleanScope::from_command(&CleanCommand {
            all: true,
            ..CleanCommand::default()
        });
        assert!(scope.sessions);
        assert!(scope.images);
        assert!(scope.caches);
    }

    #[test]
    fn cache_volume_names_only_include_implicit_volumes() {
        let caches = vec![
            CacheConfig {
                name: "first".into(),
                target: "/cache".into(),
                source: None,
                read_only: None,
            },
            CacheConfig {
                name: "host".into(),
                target: "/host".into(),
                source: Some("./cache".into()),
                read_only: None,
            },
        ];

        let names = cache_volume_names(&caches, Path::new("/tmp/workspace"));
        assert_eq!(names.len(), 1);
        assert!(names[0].contains("sbox-cache-"));
    }

    #[test]
    fn reusable_session_names_follow_runtime_defaults() {
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
        let config = Config {
            version: 1,
            runtime: Some(RuntimeConfig {
                backend: Some(BackendKind::Podman),
                rootless: Some(true),
                reuse_container: Some(true),
                container_name: None,
                pull_policy: None,
                strict_security: None,
                require_pinned_image: None,
            }),
            workspace: None,
            identity: None,
            image: None,
            environment: None,
            mounts: Vec::new(),
            caches: Vec::new(),
            secrets: Vec::new(),
            profiles,
            dispatch: IndexMap::new(),

            package_manager: None,
        };

        let names = reusable_session_names(&config, Path::new("/tmp/workspace"));
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("sbox-"));
    }
}
