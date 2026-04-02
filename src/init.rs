use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::cli::{Cli, InitCommand};
use crate::error::SboxError;

pub fn execute(cli: &Cli, command: &InitCommand) -> Result<ExitCode, SboxError> {
    let target = resolve_output_path(cli, command)?;
    if target.exists() && !command.force {
        return Err(SboxError::InitConfigExists { path: target });
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|source| SboxError::InitWrite {
            path: target.clone(),
            source,
        })?;
    }

    let preset = command.preset.as_deref().unwrap_or("generic");
    let template = render_template(preset)?;
    fs::write(&target, template).map_err(|source| SboxError::InitWrite {
        path: target.clone(),
        source,
    })?;

    println!("created {}", target.display());
    Ok(ExitCode::SUCCESS)
}

fn resolve_output_path(cli: &Cli, command: &InitCommand) -> Result<PathBuf, SboxError> {
    let cwd = std::env::current_dir().map_err(|source| SboxError::CurrentDirectory { source })?;
    let base = cli.workspace.clone().unwrap_or(cwd);

    Ok(match &command.output {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => base.join(path),
        None => base.join("sbox.yaml"),
    })
}

pub fn render_template(preset: &str) -> Result<String, SboxError> {
    let image = match preset {
        "generic" | "polyglot" => "ubuntu:24.04",
        "python" => "python:3.13-slim",
        "rust" => "rust:1-bookworm",
        "node" => "node:22-bookworm-slim",
        other => {
            return Err(SboxError::UnknownPreset {
                name: other.to_string(),
            });
        }
    };

    Ok(format!(
        "version: 1\n\nruntime:\n  backend: podman\n  rootless: true\n  reuse_container: false\n\nworkspace:\n  root: .\n  mount: /workspace\n  writable: true\n\nimage:\n  ref: {image}\n\nenvironment:\n  pass_through:\n    - TERM\n  set: {{}}\n  deny: []\n\nmounts: []\ncaches: []\nsecrets: []\n\nprofiles:\n  default:\n    mode: sandbox\n    network: off\n    writable: true\n    ports: []\n    no_new_privileges: true\n\n  host:\n    mode: host\n    network: on\n    writable: true\n    ports: []\n\ndispatch: {{}}\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::render_template;

    #[test]
    fn renders_python_template() {
        let rendered = render_template("python").expect("python preset should exist");
        assert!(rendered.contains("ref: python:3.13-slim"));
        assert!(rendered.contains("mode: sandbox"));
    }
}
