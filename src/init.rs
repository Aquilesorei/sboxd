use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};

use crate::cli::{Cli, InitCommand};
use crate::error::SboxError;

pub fn execute(cli: &Cli, command: &InitCommand) -> Result<ExitCode, SboxError> {
    if command.interactive {
        return execute_interactive(cli, command);
    }

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

// ── Interactive wizard ────────────────────────────────────────────────────────

fn execute_interactive(cli: &Cli, command: &InitCommand) -> Result<ExitCode, SboxError> {
    let target = resolve_output_path(cli, command)?;
    if target.exists() && !command.force {
        return Err(SboxError::InitConfigExists { path: target });
    }

    let theme = ColorfulTheme::default();
    println!("sbox interactive setup");
    println!("──────────────────────");
    println!("Use arrow keys to select, Enter to confirm.\n");

    // ── Simple vs Advanced ────────────────────────────────────────────────────
    let mode_idx = Select::with_theme(&theme)
        .with_prompt("Setup mode")
        .items(&[
            "simple   — package_manager preset (recommended)",
            "advanced — manual profiles and dispatch rules",
        ])
        .default(0)
        .interact()
        .map_err(|_| SboxError::CurrentDirectory {
            source: std::io::Error::other("prompt cancelled"),
        })?;

    let config = if mode_idx == 0 {
        execute_interactive_simple(&theme)?
    } else {
        execute_interactive_advanced(&theme)?
    };

    // ── Write ─────────────────────────────────────────────────────────────────
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|source| SboxError::InitWrite {
            path: target.clone(),
            source,
        })?;
    }
    fs::write(&target, &config).map_err(|source| SboxError::InitWrite {
        path: target.clone(),
        source,
    })?;

    println!("\ncreated {}", target.display());
    println!("Run `sbox plan -- <command>` to preview the resolved policy.");
    Ok(ExitCode::SUCCESS)
}

fn execute_interactive_simple(theme: &ColorfulTheme) -> Result<String, SboxError> {
    // ── Package manager ───────────────────────────────────────────────────────
    let pm_idx = Select::with_theme(theme)
        .with_prompt("Package manager")
        .items(&["npm", "yarn", "pnpm", "bun", "uv", "pip", "poetry", "cargo", "go"])
        .default(0)
        .interact()
        .map_err(|_| SboxError::CurrentDirectory {
            source: std::io::Error::other("prompt cancelled"),
        })?;
    let (pm_name, default_image) = [
        ("npm",    "node:22-bookworm-slim"),
        ("yarn",   "node:22-bookworm-slim"),
        ("pnpm",   "node:22-bookworm-slim"),
        ("bun",    "oven/bun:1"),
        ("uv",     "python:3.13-slim"),
        ("pip",    "python:3.13-slim"),
        ("poetry", "python:3.13-slim"),
        ("cargo",  "rust:1-bookworm"),
        ("go",     "golang:1.23-bookworm"),
    ][pm_idx];

    // ── Image ─────────────────────────────────────────────────────────────────
    let image: String = Input::with_theme(theme)
        .with_prompt("Container image")
        .default(default_image.to_string())
        .interact_text()
        .map_err(|_| SboxError::CurrentDirectory {
            source: std::io::Error::other("prompt cancelled"),
        })?;

    // ── Backend ───────────────────────────────────────────────────────────────
    let backend_idx = Select::with_theme(theme)
        .with_prompt("Container backend")
        .items(&["auto (detect podman or docker)", "podman", "docker"])
        .default(0)
        .interact()
        .map_err(|_| SboxError::CurrentDirectory {
            source: std::io::Error::other("prompt cancelled"),
        })?;
    let runtime_block = match backend_idx {
        1 => "runtime:\n  backend: podman\n  rootless: true\n",
        2 => "runtime:\n  backend: docker\n  rootless: false\n",
        _ => "",
    };

    let exclude_paths = default_exclude_paths(pm_name);

    Ok(format!(
        "version: 1\n\
         \n\
         {runtime_block}\
         \n\
         workspace:\n\
           mount: /workspace\n\
           writable: false\n\
           exclude_paths:\n\
         {exclude_paths}\
         \n\
         image:\n\
           ref: {image}\n\
         \n\
         environment:\n\
           pass_through:\n\
             - TERM\n\
         \n\
         package_manager:\n\
           name: {pm_name}\n"
    ))
}

fn default_exclude_paths(pm_name: &str) -> String {
    let common = vec![
        "    - \".ssh/*\"",
        "    - \".aws/*\"",
    ];
    let extras: &[&str] = match pm_name {
        "npm" | "yarn" | "pnpm" | "bun" => &[
            "    - .env",
            "    - .env.local",
            "    - .env.production",
            "    - .env.development",
            "    - .npmrc",
            "    - .netrc",
        ],
        "uv" | "pip" | "poetry" => &[
            "    - .env",
            "    - .env.local",
            "    - .netrc",
        ],
        _ => &[],
    };

    let mut lines: Vec<&str> = extras.to_vec();
    lines.extend_from_slice(&common);
    lines.join("\n") + "\n"
}

fn execute_interactive_advanced(theme: &ColorfulTheme) -> Result<String, SboxError> {
    // ── Backend ───────────────────────────────────────────────────────────────
    let backend_idx = Select::with_theme(theme)
        .with_prompt("Container backend")
        .items(&["auto (detect podman or docker)", "podman", "docker"])
        .default(0)
        .interact()
        .map_err(|_| SboxError::CurrentDirectory {
            source: std::io::Error::other("prompt cancelled"),
        })?;
    let (backend_line, rootless_line) = match backend_idx {
        1 => ("  backend: podman", "  rootless: true"),
        2 => ("  backend: docker", "  rootless: false"),
        _ => ("  # backend: auto-detected", "  rootless: true"),
    };

    // ── Preset / image ────────────────────────────────────────────────────────
    let preset_idx = Select::with_theme(theme)
        .with_prompt("Language / ecosystem")
        .items(&["node", "python", "rust", "go", "generic", "custom image"])
        .default(0)
        .interact()
        .map_err(|_| SboxError::CurrentDirectory {
            source: std::io::Error::other("prompt cancelled"),
        })?;

    let preset = ["node", "python", "rust", "go", "generic", "custom"][preset_idx];

    let (default_image, default_writable_paths, default_dispatch) = match preset {
        "node"   => ("node:22-bookworm-slim", vec!["node_modules", "package-lock.json", "dist"], node_dispatch()),
        "python" => ("python:3.13-slim",       vec![".venv"],                                   python_dispatch()),
        "rust"   => ("rust:1-bookworm",         vec!["target"],                                  rust_dispatch()),
        "go"     => ("golang:1.23-bookworm",    vec![],                                          go_dispatch()),
        _        => ("ubuntu:24.04",            vec![],                                          String::new()),
    };

    let image: String = Input::with_theme(theme)
        .with_prompt("Container image")
        .default(default_image.to_string())
        .interact_text()
        .map_err(|_| SboxError::CurrentDirectory {
            source: std::io::Error::other("prompt cancelled"),
        })?;

    // ── Network ───────────────────────────────────────────────────────────────
    let network_idx = Select::with_theme(theme)
        .with_prompt("Default network access in sandbox")
        .items(&[
            "off  — no internet (recommended for installs)",
            "on   — full internet access",
        ])
        .default(0)
        .interact()
        .map_err(|_| SboxError::CurrentDirectory {
            source: std::io::Error::other("prompt cancelled"),
        })?;
    let network = if network_idx == 0 { "off" } else { "on" };

    // ── Workspace writable paths ──────────────────────────────────────────────
    let default_wp = default_writable_paths.join(", ");
    let wp_input: String = Input::with_theme(theme)
        .with_prompt("Writable paths in workspace (comma-separated)")
        .default(default_wp)
        .allow_empty(true)
        .interact_text()
        .map_err(|_| SboxError::CurrentDirectory {
            source: std::io::Error::other("prompt cancelled"),
        })?;
    let writable_paths: Vec<String> = wp_input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // ── Dispatch rules ────────────────────────────────────────────────────────
    let add_dispatch = if !default_dispatch.is_empty() {
        Confirm::with_theme(theme)
            .with_prompt(format!("Add default dispatch rules for {preset}?"))
            .default(true)
            .interact()
            .map_err(|_| SboxError::CurrentDirectory {
                source: std::io::Error::other("prompt cancelled"),
            })?
    } else {
        false
    };

    // ── Render ────────────────────────────────────────────────────────────────
    let writable_paths_yaml = if writable_paths.is_empty() {
        "    []".to_string()
    } else {
        writable_paths
            .iter()
            .map(|p| format!("    - {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let workspace_writable = writable_paths.is_empty();
    let dispatch_section = if add_dispatch {
        format!("dispatch:\n{default_dispatch}")
    } else {
        "dispatch: {}".to_string()
    };

    Ok(format!("version: 1

runtime:
{backend_line}
{rootless_line}

workspace:
  root: .
  mount: /workspace
  writable: {workspace_writable}
  writable_paths:
{writable_paths_yaml}
  exclude_paths:
    - .env
    - .env.local
    - .env.production
    - .env.development
    - \"*.pem\"
    - \"*.key\"
    - .npmrc
    - .netrc
    - \".ssh/*\"
    - \".aws/*\"

image:
  ref: {image}

environment:
  pass_through:
    - TERM
  set: {{}}
  deny: []

profiles:
  default:
    mode: sandbox
    network: {network}
    writable: true
    no_new_privileges: true

{dispatch_section}
"))
}

// ── Default dispatch rules per preset (advanced mode) ────────────────────────

fn node_dispatch() -> String {
    "  npm-install:\n    match:\n      - \"npm install*\"\n      - \"npm ci\"\n    profile: default\n  \
     yarn-install:\n    match:\n      - \"yarn install*\"\n    profile: default\n  \
     pnpm-install:\n    match:\n      - \"pnpm install*\"\n    profile: default\n"
        .to_string()
}

fn python_dispatch() -> String {
    "  pip-install:\n    match:\n      - \"pip install*\"\n      - \"pip3 install*\"\n    profile: default\n  \
     uv-sync:\n    match:\n      - \"uv sync*\"\n    profile: default\n  \
     poetry-install:\n    match:\n      - \"poetry install*\"\n    profile: default\n"
        .to_string()
}

fn rust_dispatch() -> String {
    "  cargo-build:\n    match:\n      - \"cargo build*\"\n      - \"cargo check*\"\n    profile: default\n"
        .to_string()
}

fn go_dispatch() -> String {
    "  go-get:\n    match:\n      - \"go get*\"\n      - \"go mod download*\"\n    profile: default\n"
        .to_string()
}

// ── Non-interactive (--preset) ────────────────────────────────────────────────

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
    match preset {
        "node" => Ok(
"version: 1

workspace:
  mount: /workspace
  writable: false
  exclude_paths:
    - .env
    - .env.local
    - .env.production
    - .env.development
    - .npmrc
    - .netrc
    - \".ssh/*\"
    - \".aws/*\"

image:
  ref: node:22-bookworm-slim

environment:
  pass_through:
    - TERM

package_manager:
  name: npm
".to_string()),

        "python" => Ok(
"version: 1

workspace:
  mount: /workspace
  writable: false
  exclude_paths:
    - .env
    - .env.local
    - .netrc
    - \".ssh/*\"
    - \".aws/*\"

image:
  ref: python:3.13-slim

environment:
  pass_through:
    - TERM

package_manager:
  name: uv
".to_string()),

        "rust" => Ok(
"version: 1

workspace:
  mount: /workspace
  writable: false
  exclude_paths:
    - \".ssh/*\"
    - \".aws/*\"

image:
  ref: rust:1-bookworm

environment:
  pass_through:
    - TERM

package_manager:
  name: cargo
".to_string()),

        "go" => Ok(
"version: 1

workspace:
  mount: /workspace
  writable: false
  exclude_paths:
    - \".ssh/*\"
    - \".aws/*\"

image:
  ref: golang:1.23-bookworm

environment:
  pass_through:
    - TERM

package_manager:
  name: go
".to_string()),

        "generic" | "polyglot" => Ok(
"version: 1

runtime:
  backend: podman
  rootless: true

workspace:
  root: .
  mount: /workspace
  writable: true
  exclude_paths:
    - \".ssh/*\"
    - \".aws/*\"

image:
  ref: ubuntu:24.04

environment:
  pass_through:
    - TERM
  set: {}
  deny: []

profiles:
  default:
    mode: sandbox
    network: off
    writable: true
    no_new_privileges: true

  host:
    mode: host
    network: on
    writable: true

dispatch: {}
".to_string()),

        other => Err(SboxError::UnknownPreset {
            name: other.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::render_template;

    #[test]
    fn renders_node_template_with_package_manager() {
        let rendered = render_template("node").expect("node preset should exist");
        assert!(rendered.contains("ref: node:22-bookworm-slim"));
        assert!(rendered.contains("package_manager:"));
        assert!(rendered.contains("name: npm"));
        assert!(!rendered.contains("profiles:"));
    }

    #[test]
    fn renders_python_template_with_package_manager() {
        let rendered = render_template("python").expect("python preset should exist");
        assert!(rendered.contains("ref: python:3.13-slim"));
        assert!(rendered.contains("name: uv"));
    }

    #[test]
    fn renders_rust_template_with_package_manager() {
        let rendered = render_template("rust").expect("rust preset should exist");
        assert!(rendered.contains("ref: rust:1-bookworm"));
        assert!(rendered.contains("name: cargo"));
    }

    #[test]
    fn renders_generic_template_with_profiles() {
        let rendered = render_template("generic").expect("generic preset should exist");
        assert!(rendered.contains("profiles:"));
        assert!(!rendered.contains("package_manager:"));
    }
}
