use std::process::{Command, ExitCode, Stdio};

use crate::cli::{Cli, OutputFormat, StatusCommand};
use crate::config::{LoadOptions, load_config};
use crate::error::SboxError;

#[derive(Debug)]
struct SessionInfo {
    name: String,
    image: String,
    status: String,
    created: String,
}

pub fn execute(cli: &Cli, command: &StatusCommand) -> Result<ExitCode, SboxError> {
    let prefix = if command.all {
        "sbox-".to_string()
    } else {
        // Scope to the current workspace by deriving its hash prefix.
        match load_config(&LoadOptions {
            workspace: cli.workspace.clone(),
            config: cli.config.clone(),
        }) {
            Ok(loaded) => {
                let hash = stable_hash(&loaded.workspace_root.display().to_string());
                format!("sbox-{hash}")
            }
            Err(_) => "sbox-".to_string(),
        }
    };

    let sessions = list_sessions(&prefix)?;

    match cli.output_format {
        OutputFormat::Json => print_json(&sessions),
        OutputFormat::Text => print_text(&sessions, &prefix, command.all),
    }

    Ok(ExitCode::SUCCESS)
}

fn list_sessions(prefix: &str) -> Result<Vec<SessionInfo>, SboxError> {
    let output = Command::new("podman")
        .args([
            "ps",
            "-a",
            "--format",
            "{{.Names}}\t{{.Image}}\t{{.Status}}\t{{.Created}}",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|source| SboxError::BackendUnavailable {
            backend: "podman".to_string(),
            source,
        })?;

    let sessions = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| {
            line.split('\t')
                .next()
                .map(|name| name.starts_with(prefix))
                .unwrap_or(false)
        })
        .map(|line| {
            let mut parts = line.splitn(4, '\t');
            SessionInfo {
                name: parts.next().unwrap_or("").to_string(),
                image: parts.next().unwrap_or("").to_string(),
                status: parts.next().unwrap_or("").to_string(),
                created: parts.next().unwrap_or("").to_string(),
            }
        })
        .collect();

    Ok(sessions)
}

fn print_text(sessions: &[SessionInfo], prefix: &str, all: bool) {
    if sessions.is_empty() {
        if all {
            println!("no sbox-managed containers found on this host");
        } else {
            println!(
                "no active sessions for this workspace (prefix: {prefix})\n\
                 use `sbox status --all` to see all workspaces"
            );
        }
        return;
    }

    println!(
        "{:<40}  {:<30}  {:<20}  {}",
        "SESSION", "IMAGE", "STATUS", "CREATED"
    );
    println!("{}", "-".repeat(100));
    for s in sessions {
        println!(
            "{:<40}  {:<30}  {:<20}  {}",
            s.name, s.image, s.status, s.created
        );
    }
}

fn print_json(sessions: &[SessionInfo]) {
    let json: Vec<serde_json::Value> = sessions
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "image": s.image,
                "status": s.status,
                "created": s.created,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}

fn stable_hash(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
