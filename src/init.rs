use std::fs;
use std::path::{Path, PathBuf};
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

fn detect_dockerfile(cwd: &Path) -> Option<String> {
    for name in &[
        "Dockerfile",
        "Dockerfile.dev",
        "Dockerfile.local",
        "dockerfile",
    ] {
        if cwd.join(name).exists() {
            return Some(name.to_string());
        }
    }
    None
}

/// Well-known infrastructure/sidecar image name fragments to skip when scanning compose files.
/// We want the application service image, not postgres/redis/etc.
const COMPOSE_SIDECAR_PREFIXES: &[&str] = &[
    "postgres",
    "mysql",
    "mariadb",
    "mongo",
    "redis",
    "rabbitmq",
    "elasticsearch",
    "kibana",
    "grafana",
    "prometheus",
    "influxdb",
    "nginx",
    "traefik",
    "caddy",
    "haproxy",
    "zookeeper",
    "kafka",
    "memcached",
    "vault",
];

fn detect_compose_image(cwd: &Path) -> Option<String> {
    for name in &[
        "compose.yaml",
        "compose.yml",
        "docker-compose.yml",
        "docker-compose.yaml",
    ] {
        let path = cwd.join(name);
        if !path.exists() {
            continue;
        }
        // Extract image values from the compose file — fast heuristic, no full YAML parse.
        // Skip well-known infrastructure images (databases, caches, proxies) to avoid
        // suggesting `postgres:16` as the app image when the db service appears first.
        if let Ok(text) = fs::read_to_string(&path) {
            for line in text.lines() {
                let t = line.trim();
                if let Some(rest) = t.strip_prefix("image:") {
                    let img = rest.trim().trim_matches('"').trim_matches('\'');
                    if img.is_empty() {
                        continue;
                    }
                    let img_lower = img.to_lowercase();
                    let is_sidecar = COMPOSE_SIDECAR_PREFIXES
                        .iter()
                        .any(|p| img_lower.starts_with(p));
                    if !is_sidecar {
                        return Some(img.to_string());
                    }
                }
            }
        }
    }
    None
}

fn execute_interactive_simple(theme: &ColorfulTheme) -> Result<String, SboxError> {
    let cwd = std::env::current_dir().map_err(|source| SboxError::CurrentDirectory { source })?;

    // ── Detect existing Docker infrastructure ─────────────────────────────────
    let found_dockerfile = detect_dockerfile(&cwd);
    let found_compose_image = detect_compose_image(&cwd);

    // ── Package manager ───────────────────────────────────────────────────────
    let pm_idx = Select::with_theme(theme)
        .with_prompt("Package manager")
        .items(&[
            "npm", "yarn", "pnpm", "bun", "uv", "pip", "poetry", "cargo", "go",
        ])
        .default(0)
        .interact()
        .map_err(|_| SboxError::CurrentDirectory {
            source: std::io::Error::other("prompt cancelled"),
        })?;
    let (pm_name, stock_image) = [
        ("npm", "node:22-bookworm-slim"),
        ("yarn", "node:22-bookworm-slim"),
        ("pnpm", "node:22-bookworm-slim"),
        ("bun", "oven/bun:1"),
        ("uv", "ghcr.io/astral-sh/uv:python3.13-bookworm-slim"),
        ("pip", "python:3.13-slim"),
        ("poetry", "python:3.13-slim"),
        ("cargo", "rust:1-bookworm"),
        ("go", "golang:1.23-bookworm"),
    ][pm_idx];

    // ── Image — prefer existing Docker infrastructure over stock public images ─
    let image_block: String = if let Some(ref dockerfile) = found_dockerfile {
        let use_it = Confirm::with_theme(theme)
            .with_prompt(format!(
                "Found `{dockerfile}` — use it as the container image?"
            ))
            .default(true)
            .interact()
            .map_err(|_| SboxError::CurrentDirectory {
                source: std::io::Error::other("prompt cancelled"),
            })?;
        if use_it {
            format!("image:\n  build: {dockerfile}\n")
        } else {
            let img = prompt_image(theme, stock_image)?;
            format!("image:\n  ref: {img}\n")
        }
    } else if let Some(ref compose_image) = found_compose_image {
        let use_it = Confirm::with_theme(theme)
            .with_prompt(format!(
                "Found image `{compose_image}` in compose file — use it?"
            ))
            .default(true)
            .interact()
            .map_err(|_| SboxError::CurrentDirectory {
                source: std::io::Error::other("prompt cancelled"),
            })?;
        if use_it {
            format!("image:\n  ref: {compose_image}\n")
        } else {
            let img = prompt_image(theme, stock_image)?;
            format!("image:\n  ref: {img}\n")
        }
    } else {
        let img = prompt_image(theme, stock_image)?;
        format!("image:\n  ref: {img}\n")
    };

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
        "version: 1

{runtime_block}
workspace:
  mount: /workspace
  writable: false
  exclude_paths:
{exclude_paths}
{image_block}
environment:
  pass_through:
    - TERM

package_manager:
  name: {pm_name}
"
    ))
}

fn prompt_image(theme: &ColorfulTheme, default: &str) -> Result<String, SboxError> {
    Input::with_theme(theme)
        .with_prompt("Container image")
        .default(default.to_string())
        .interact_text()
        .map_err(|_| SboxError::CurrentDirectory {
            source: std::io::Error::other("prompt cancelled"),
        })
}

fn default_exclude_paths(pm_name: &str) -> String {
    let common = vec!["    - \".ssh/*\"", "    - \".aws/*\""];
    let extras: &[&str] = match pm_name {
        "npm" | "yarn" | "pnpm" | "bun" => &[
            "    - .env",
            "    - .env.local",
            "    - .env.production",
            "    - .env.development",
            "    - .npmrc",
            "    - .netrc",
        ],
        "uv" | "pip" | "poetry" => &["    - .env", "    - .env.local", "    - .netrc"],
        _ => &[],
    };

    let mut lines: Vec<&str> = extras.to_vec();
    lines.extend_from_slice(&common);
    lines.join("\n") + "\n"
}

fn execute_interactive_advanced(theme: &ColorfulTheme) -> Result<String, SboxError> {
    let cwd = std::env::current_dir().map_err(|source| SboxError::CurrentDirectory { source })?;
    let found_dockerfile = detect_dockerfile(&cwd);
    let found_compose_image = detect_compose_image(&cwd);

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
    // Build the ecosystem list, prepending detected local infrastructure.
    let mut image_choices: Vec<String> = Vec::new();
    if let Some(ref df) = found_dockerfile {
        image_choices.push(format!("existing Dockerfile ({df})"));
    }
    if let Some(ref img) = found_compose_image {
        image_choices.push(format!("image from compose ({img})"));
    }
    image_choices.extend_from_slice(&[
        "node".into(),
        "python".into(),
        "rust".into(),
        "go".into(),
        "generic".into(),
        "custom image".into(),
    ]);

    let image_idx = Select::with_theme(theme)
        .with_prompt("Container image source")
        .items(&image_choices)
        .default(0)
        .interact()
        .map_err(|_| SboxError::CurrentDirectory {
            source: std::io::Error::other("prompt cancelled"),
        })?;

    // Resolve offset caused by prepended Dockerfile/compose choices.
    let offset = (found_dockerfile.is_some() as usize) + (found_compose_image.is_some() as usize);
    let ecosystem_names = ["node", "python", "rust", "go", "generic", "custom"];

    let (image_yaml, preset, default_writable_paths, default_dispatch) = if found_dockerfile
        .is_some()
        && image_idx == 0
    {
        let df = found_dockerfile.as_deref().unwrap();
        (
            format!("image:\n  build: {df}"),
            "custom",
            vec![],
            String::new(),
        )
    } else if found_compose_image.is_some() && image_idx == (found_dockerfile.is_some() as usize) {
        let img = found_compose_image.as_deref().unwrap();
        (
            format!("image:\n  ref: {img}"),
            "custom",
            vec![],
            String::new(),
        )
    } else {
        let preset = ecosystem_names[image_idx - offset];
        let (default_image, writable, dispatch) = match preset {
            "node" => (
                "node:22-bookworm-slim",
                vec!["node_modules", "package-lock.json", "dist"],
                node_dispatch(),
            ),
            "python" => ("python:3.13-slim", vec![".venv"], python_dispatch()),
            "rust" => ("rust:1-bookworm", vec!["target"], rust_dispatch()),
            "go" => ("golang:1.23-bookworm", vec![], go_dispatch()),
            _ => ("ubuntu:24.04", vec![], String::new()),
        };
        let img = prompt_image(theme, default_image)?;
        (format!("image:\n  ref: {img}"), preset, writable, dispatch)
    };

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

    Ok(format!(
        "version: 1

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

{image_yaml}

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
"
    ))
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
        "node" => Ok("version: 1

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
"
        .to_string()),

        "python" => Ok("version: 1

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
  ref: ghcr.io/astral-sh/uv:python3.13-bookworm-slim

environment:
  pass_through:
    - TERM

package_manager:
  name: uv
"
        .to_string()),

        "rust" => Ok("version: 1

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
"
        .to_string()),

        "go" => Ok("version: 1

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
"
        .to_string()),

        "generic" | "polyglot" => Ok("version: 1

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
"
        .to_string()),

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
        assert!(rendered.contains("ghcr.io/astral-sh/uv:python3.13-bookworm-slim"));
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
