use std::process::{Command, ExitCode, Stdio};

use crate::cli::{Cli, LogsCommand};
use crate::config::{LoadOptions, load_config};
use crate::error::SboxError;

pub fn execute(cli: &Cli, command: &LogsCommand) -> Result<ExitCode, SboxError> {
    let loaded = load_config(&LoadOptions {
        workspace: cli.workspace.clone(),
        config: cli.config.clone(),
    })?;

    let workspace_hash = stable_hash(&loaded.workspace_root.display().to_string());

    // Resolve the session name: explicit profile arg, or pick the most recent running session.
    let session_name = if let Some(profile) = &command.profile {
        // Build the session name using the same convention as clean.rs / resolve.rs.
        let template = loaded
            .config
            .runtime
            .as_ref()
            .and_then(|rt| rt.container_name.as_ref());
        resolve_session_name(template, &workspace_hash, profile)
    } else {
        find_most_recent_session(&workspace_hash)?
            .ok_or_else(|| SboxError::ConfigValidation {
                message: format!(
                    "no running sbox session found for this workspace.\n\
                     Start one with `sbox run -- <command>` using a profile with `reuse_container: true`,\n\
                     or pass a profile name: `sbox logs <profile>`"
                ),
            })?
    };

    let tail_str = command.tail.to_string();
    let mut args = vec!["logs", "--tail", &tail_str];
    if command.follow {
        args.push("--follow");
    }
    args.push(&session_name);

    let status = Command::new("podman")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    Ok(crate::exec::status_to_exit_code(status))
}

fn find_most_recent_session(workspace_hash: &str) -> Result<Option<String>, SboxError> {
    let prefix = format!("sbox-{workspace_hash}");

    let output = Command::new("podman")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("name={prefix}"),
            "--format",
            "{{.Names}}",
            "--sort",
            "created",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .last()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from))
}

fn resolve_session_name(
    template: Option<&String>,
    workspace_hash: &str,
    profile_name: &str,
) -> String {
    sanitize(
        &template
            .map(|t| {
                t.replace("{profile}", profile_name)
                    .replace("{workspace_hash}", workspace_hash)
            })
            .unwrap_or_else(|| format!("sbox-{workspace_hash}-{profile_name}")),
    )
}

fn sanitize(name: &str) -> String {
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
