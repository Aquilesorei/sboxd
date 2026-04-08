use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};

use crate::cli::{Cli, InitCommand};
use crate::error::SboxError;

pub fn execute(cli: &Cli, command: &InitCommand) -> Result<ExitCode, SboxError> {
    if command.interactive {
        return execute_interactive(cli, command);
    }

    if command.from_lockfile {
        return execute_from_lockfile(cli, command);
    }

    let target = resolve_output_path(cli, command)?;
    if target.exists() && !command.force {
        return Err(SboxError::InitConfigExists { path: target });
    }

    // New: If no preset is provided, perform smart auto-detection.
    if command.preset.is_none() {
        return execute_smart_init(cli, command, &target);
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|source| SboxError::InitWrite {
            path: target.clone(),
            source,
        })?;
    }

    let preset = command.preset.as_deref().unwrap_or("generic");
    let mut template = render_template(preset)?;
    template = pin_digest_in_template(template);
    fs::write(&target, template).map_err(|source| SboxError::InitWrite {
        path: target.clone(),
        source,
    })?;

    println!("created {}", target.display());
    Ok(ExitCode::SUCCESS)
}

fn pin_digest_in_template(template: String) -> String {
    // Only apply to templates that set `image.ref` and do not already include a digest.
    if template.contains("\n  digest:") {
        return template;
    }

    let needle = "\nimage:\n  ref: ";
    let Some(start) = template.find(needle) else {
        return template;
    };

    let ref_start = start + needle.len();
    let Some(rest) = template.get(ref_start..) else {
        return template;
    };
    let Some(ref_end_rel) = rest.find('\n') else {
        return template;
    };
    let reference = rest[..ref_end_rel].trim();
    if reference.is_empty() {
        return template;
    }

    let Some(digest) = try_fetch_image_digest(reference) else {
        eprintln!(
            "sbox init: warning: could not fetch digest for `{}`; config will use an unpinned image reference",
            reference
        );
        return template;
    };

    let insert_at = ref_start + ref_end_rel;
    let mut out = String::with_capacity(template.len() + digest.len() + 16);
    out.push_str(&template[..insert_at]);
    out.push_str(&format!("\n  digest: {digest}"));
    out.push_str(&template[insert_at..]);
    out
}

fn apply_runtime_compose(config: String, compose_file: &str, services: &[String]) -> String {
    if services.is_empty() || config.contains("\n  compose:\n") {
        return config;
    }

    let services_yaml = services
        .iter()
        .map(|s| format!("      - {s}"))
        .collect::<Vec<_>>()
        .join("\n");

    config.replacen(
        "runtime:\n",
        &format!(
            "runtime:\n  compose:\n    file: {compose_file}\n    services:\n{services_yaml}\n"
        ),
        1,
    )
}

fn apply_runtime_backend(config: String, kind: &str, rootless: Option<bool>) -> String {
    let config = config
        .replacen("  backend: podman", &format!("  backend: {kind}"), 1)
        .replacen("  backend: docker", &format!("  backend: {kind}"), 1);

    match rootless {
        Some(rootless) => {
            let rootless = if rootless { "true" } else { "false" };
            let map_user = if rootless == "true" { "true" } else { "false" };
            let config = config
                .replacen("  rootless: true", &format!("  rootless: {rootless}"), 1)
                .replacen("  rootless: false", &format!("  rootless: {rootless}"), 1);

            config
                .replacen("  map_user: true", &format!("  map_user: {map_user}"), 1)
                .replacen("  map_user: false", &format!("  map_user: {map_user}"), 1)
        }
        None => config,
    }
}

#[cfg(test)]
mod pin_digest_tests {
    use super::pin_digest_in_template;

    #[test]
    fn noop_when_digest_already_present() {
        let input = "version: 1\n\nimage:\n  ref: ubuntu:24.04\n  digest: sha256:deadbeef\n";
        let out = pin_digest_in_template(input.to_string());
        assert_eq!(out, input);
    }

    #[test]
    fn noop_when_no_image_ref() {
        let input = "version: 1\n\nimage:\n  build: Dockerfile\n";
        let out = pin_digest_in_template(input.to_string());
        assert_eq!(out, input);
    }
}

fn execute_smart_init(
    _cli: &Cli,
    _command: &InitCommand,
    target: &PathBuf,
) -> Result<ExitCode, SboxError> {
    let cwd = std::env::current_dir().map_err(|source| SboxError::CurrentDirectory { source })?;
    let project = scan_project_context(&cwd);
    let preset = recommended_package_manager(&project.package_manager).unwrap_or("generic");

    println!("detected project type → using preset: {preset}");

    let mut config = if preset == "generic" {
        render_template("generic")?
    } else {
        let stock_image = match preset {
            "npm" | "yarn" | "pnpm" => "node:22-bookworm-slim",
            "bun" => "oven/bun:1",
            "uv" => "ghcr.io/astral-sh/uv:python3.13-bookworm-slim",
            "pip" | "poetry" => "python:3.13-slim",
            "cargo" => "rust:1-bookworm",
            "go" => "golang:1.23-bookworm",
            _ => "ubuntu:24.04",
        };

        let mut image_line = format!("  ref: {stock_image}");
        if let Some(ref container_definition) = project.container_definition {
            println!(
                "found `{}` → using it for image build",
                container_definition.path
            );
            image_line = format!("  build: {}", container_definition.path);
        }

        let eco = match preset {
            "npm" | "yarn" | "pnpm" | "bun" => "node",
            "uv" | "pip" | "poetry" => "python",
            "cargo" => "rust",
            "go" => "go",
            _ => "generic",
        };

        let mut c = render_template_with_image_line(eco, &image_line).expect("known preset");

        // Correctly patch the package manager name
        let default_pm = match eco {
            "node" => "npm",
            "python" => "uv",
            "rust" => "cargo",
            "go" => "go",
            _ => eco,
        };
        c = c.replace(&format!("name: {default_pm}"), &format!("name: {preset}"));
        c
    };

    // Add compose info if found
    if let Some(compose) = &project.compose {
        println!("found `{}` → importing sidecars", compose.compose_file);
        if !compose.sidecar_services.is_empty() {
            config =
                apply_runtime_compose(config, &compose.compose_file, &compose.sidecar_services);
        }
    }

    // Add backend preference if detected
    if let Some(backend) = project.backend {
        config = apply_runtime_backend(config, backend.kind, backend.rootless);
    }

    config = pin_digest_in_template(config);

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|source| SboxError::InitWrite {
            path: target.clone(),
            source,
        })?;
    }
    fs::write(target, config).map_err(|source| SboxError::InitWrite {
        path: target.clone(),
        source,
    })?;

    println!("created {}", target.display());
    Ok(ExitCode::SUCCESS)
}

fn execute_from_lockfile(cli: &Cli, command: &InitCommand) -> Result<ExitCode, SboxError> {
    let cwd = std::env::current_dir().map_err(|source| SboxError::CurrentDirectory { source })?;

    let detected = detect_lockfile_preset(&cwd);
    let preset = detected.ok_or_else(|| SboxError::ConfigValidation {
        message: "no recognised lockfile found in the current directory. \
                  Supported: package-lock.json, yarn.lock, pnpm-lock.yaml, bun.lock(b), \
                  uv.lock, requirements.txt, poetry.lock, Cargo.lock, go.sum, \
                  composer.lock, Gemfile.lock"
            .to_string(),
    })?;

    println!("detected lockfile → using preset: {preset}");

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

    let mut template = render_template(preset)?;
    template = pin_digest_in_template(template);
    fs::write(&target, template).map_err(|source| SboxError::InitWrite {
        path: target.clone(),
        source,
    })?;

    println!("created {}", target.display());
    Ok(ExitCode::SUCCESS)
}

/// Maps a lockfile filename to its preset name. Checked in priority order — more specific
/// lockfiles (uv.lock, poetry.lock) are checked before generic ones (requirements.txt).
fn detect_lockfile_preset(dir: &Path) -> Option<&'static str> {
    const LOCKFILE_MAP: &[(&str, &str)] = &[
        ("package-lock.json", "npm"),
        ("npm-shrinkwrap.json", "npm"),
        ("yarn.lock", "yarn"),
        ("pnpm-lock.yaml", "pnpm"),
        ("bun.lockb", "bun"),
        ("bun.lock", "bun"),
        ("uv.lock", "uv"),
        ("poetry.lock", "poetry"),
        ("requirements.txt", "pip"),
        ("Cargo.lock", "cargo"),
        ("go.sum", "go"),
        ("composer.lock", "composer"),
        ("Gemfile.lock", "bundler"),
    ];

    let mut detected = None;
    for &(filename, preset) in LOCKFILE_MAP {
        if dir.join(filename).exists() {
            detected = Some(preset);
            break;
        }
    }

    let preset = detected?;

    // Smart override: if we detected uv but there is a Dockerfile using pip+requirements.txt, favor pip.
    if preset == "uv" && dir.join("requirements.txt").exists() {
        if let Some(container_definition) = detect_container_definition(dir) {
            if let Ok(content) = fs::read_to_string(dir.join(container_definition.path)) {
                if content.contains("pip install") && content.contains("requirements.txt") {
                    return Some("pip");
                }
            }
        }
    }

    Some(preset)
}

// ── Interactive wizard ────────────────────────────────────────────────────────

fn execute_interactive(cli: &Cli, command: &InitCommand) -> Result<ExitCode, SboxError> {
    let target = resolve_output_path(cli, command)?;
    if target.exists() && !command.force {
        return Err(SboxError::InitConfigExists { path: target });
    }

    let cwd = std::env::current_dir().map_err(|source| SboxError::CurrentDirectory { source })?;
    let project = scan_project_context(&cwd);

    let theme = ColorfulTheme::default();
    println!("sbox interactive setup");
    println!("──────────────────────");

    if !command.all {
        if let Some(p) = project.package_manager.detected {
            println!("→ detected project type: {p}");
        }
        if let Some(ref container_definition) = project.container_definition {
            println!(
                "→ found container definition: {}",
                container_definition.path
            );
        }
        if let Some(ref c) = project.compose {
            println!("→ found compose file: {}", c.compose_file);
        }
        if let Some(backend) = project.backend {
            println!("→ inferred backend: {}", backend.kind);
            if let Some(rootless) = backend.rootless {
                println!("→ detected rootless mode: {rootless}");
            }
        }
    } else {
        println!("→ `--all` mode: prompting from scratch without auto-applying detections");
    }
    println!("Use arrow keys to select, Enter to confirm.\n");

    if command.all {
        let mut config = execute_interactive_advanced(&theme, &blank_project_context())?;
        config = maybe_append_command_aliases(&theme, config)?;
        return write_init_config(target, &config);
    }

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
        execute_interactive_simple(&theme, &project)?
    } else {
        execute_interactive_advanced(&theme, &project)?
    };

    let config = maybe_append_command_aliases(&theme, config)?;

    write_init_config(target, &config)
}

fn write_init_config(target: PathBuf, config: &str) -> Result<ExitCode, SboxError> {
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

/// Well-known service names that are almost certainly the primary application service.
const APP_SERVICE_NAMES: &[&str] = &[
    "app",
    "web",
    "api",
    "backend",
    "server",
    "frontend",
    "application",
    "service",
];

struct ContainerDefinition {
    path: String,
    backend_hint: &'static str,
}

struct PackageManagerContext {
    detected: Option<&'static str>,
    choices: Vec<&'static str>,
}

struct ProjectContext {
    package_manager: PackageManagerContext,
    container_definition: Option<ContainerDefinition>,
    compose: Option<ComposeInfo>,
    backend: Option<BackendContext>,
}

#[derive(Clone, Copy)]
struct BackendContext {
    kind: &'static str,
    rootless: Option<bool>,
}

struct ComposeInfo {
    image: Option<String>,
    ports: Vec<String>,
    env: Vec<String>,
    sidecar_services: Vec<String>,
    compose_file: String,
}

struct CommandAliasPrompt {
    name: String,
    run: Vec<String>,
    profile: Option<String>,
    description: Option<String>,
}

fn scan_project_context(cwd: &Path) -> ProjectContext {
    let container_definition = detect_container_definition(cwd);
    let compose = detect_compose_info(cwd);
    let package_manager = detect_package_manager_context(cwd);
    let backend = detect_backend_context(container_definition.as_ref(), compose.as_ref());

    ProjectContext {
        package_manager,
        container_definition,
        compose,
        backend,
    }
}

fn blank_project_context() -> ProjectContext {
    ProjectContext {
        package_manager: PackageManagerContext {
            detected: None,
            choices: vec![
                "npm", "yarn", "pnpm", "bun", "uv", "pip", "poetry", "cargo", "go",
            ],
        },
        container_definition: None,
        compose: None,
        backend: None,
    }
}

fn detect_package_manager_context(dir: &Path) -> PackageManagerContext {
    const ALL: &[&str] = &[
        "npm", "yarn", "pnpm", "bun", "uv", "pip", "poetry", "cargo", "go",
    ];
    const NODE: &[&str] = &["npm", "yarn", "pnpm", "bun"];
    const PYTHON: &[&str] = &["uv", "pip", "poetry"];

    let detected = detect_lockfile_preset(dir).filter(|pm| ALL.contains(pm));
    if let Some(pm) = detected {
        return PackageManagerContext {
            detected: Some(pm),
            choices: vec![pm],
        };
    }

    let package_manager_field = detect_package_json_package_manager(dir);
    if let Some(pm) = package_manager_field {
        return PackageManagerContext {
            detected: Some(pm),
            choices: vec![pm],
        };
    }

    if let Some(container_definition) = detect_container_definition(dir) {
        if let Some(pm) =
            detect_package_manager_from_container_definition(dir, &container_definition)
        {
            return PackageManagerContext {
                detected: Some(pm),
                choices: vec![pm],
            };
        }
    }

    if dir.join("package.json").exists() {
        return PackageManagerContext {
            detected: None,
            choices: NODE.to_vec(),
        };
    }

    if dir.join("pyproject.toml").exists()
        || dir.join("requirements.txt").exists()
        || dir.join("requirements-dev.txt").exists()
        || dir.join("setup.py").exists()
    {
        return PackageManagerContext {
            detected: None,
            choices: PYTHON.to_vec(),
        };
    }

    if dir.join("Cargo.toml").exists() {
        return PackageManagerContext {
            detected: Some("cargo"),
            choices: vec!["cargo"],
        };
    }

    if dir.join("go.mod").exists() {
        return PackageManagerContext {
            detected: Some("go"),
            choices: vec!["go"],
        };
    }

    PackageManagerContext {
        detected: None,
        choices: ALL.to_vec(),
    }
}

fn recommended_package_manager(context: &PackageManagerContext) -> Option<&'static str> {
    context
        .detected
        .or_else(|| context.choices.first().copied())
}

fn detect_package_json_package_manager(dir: &Path) -> Option<&'static str> {
    let package_json = dir.join("package.json");
    let contents = fs::read_to_string(package_json).ok()?;
    let lowered = contents.to_lowercase();

    if lowered.contains("\"packagemanager\"") {
        if lowered.contains("\"packagemanager\":\"pnpm@")
            || lowered.contains("\"packagemanager\": \"pnpm@")
        {
            return Some("pnpm");
        }
        if lowered.contains("\"packagemanager\":\"yarn@")
            || lowered.contains("\"packagemanager\": \"yarn@")
        {
            return Some("yarn");
        }
        if lowered.contains("\"packagemanager\":\"bun@")
            || lowered.contains("\"packagemanager\": \"bun@")
        {
            return Some("bun");
        }
        if lowered.contains("\"packagemanager\":\"npm@")
            || lowered.contains("\"packagemanager\": \"npm@")
        {
            return Some("npm");
        }
    }

    None
}

fn detect_package_manager_from_container_definition(
    dir: &Path,
    container_definition: &ContainerDefinition,
) -> Option<&'static str> {
    let contents = fs::read_to_string(dir.join(&container_definition.path))
        .ok()?
        .to_lowercase();

    if contents.contains(" pnpm ")
        || contents.contains("pnpm install")
        || contents.contains("corepack enable pnpm")
    {
        return Some("pnpm");
    }
    if contents.contains(" yarn ")
        || contents.contains("yarn install")
        || contents.contains("corepack enable yarn")
    {
        return Some("yarn");
    }
    if contents.contains(" bun ")
        || contents.contains("bun install")
        || contents.contains("oven/bun")
    {
        return Some("bun");
    }
    if contents.contains("npm ci") || contents.contains("npm install") || contents.contains(" npm ")
    {
        return Some("npm");
    }
    if contents.contains("uv sync")
        || contents.contains("uv pip")
        || contents.contains("ghcr.io/astral-sh/uv")
        || contents.contains(" astral-sh/uv")
    {
        return Some("uv");
    }
    if contents.contains("poetry install") || contents.contains(" poetry ") {
        return Some("poetry");
    }
    if contents.contains("pip install") || contents.contains("python -m pip") {
        return Some("pip");
    }
    if contents.contains("cargo build")
        || contents.contains("cargo check")
        || contents.contains("cargo chef")
        || contents.contains(" rust:")
    {
        return Some("cargo");
    }
    if contents.contains("go mod download")
        || contents.contains("go build")
        || contents.contains(" golang:")
    {
        return Some("go");
    }

    None
}

fn detect_container_definition(cwd: &Path) -> Option<ContainerDefinition> {
    for name in &[
        "Containerfile",
        "Containerfile.dev",
        "Containerfile.local",
        "containerfile",
    ] {
        if cwd.join(name).exists() {
            return Some(ContainerDefinition {
                path: name.to_string(),
                backend_hint: "podman",
            });
        }
    }

    for name in &[
        "Dockerfile",
        "Dockerfile.dev",
        "Dockerfile.local",
        "dockerfile",
    ] {
        if cwd.join(name).exists() {
            return Some(ContainerDefinition {
                path: name.to_string(),
                backend_hint: "docker",
            });
        }
    }

    None
}

fn detect_backend_context(
    container_definition: Option<&ContainerDefinition>,
    compose: Option<&ComposeInfo>,
) -> Option<BackendContext> {
    let kind = if let Some(compose) = compose {
        if compose.compose_file.starts_with("podman-compose") {
            Some("podman")
        } else if compose.compose_file.starts_with("docker-compose") {
            Some("docker")
        } else {
            None
        }
    } else if let Some(container_definition) = container_definition {
        Some(container_definition.backend_hint)
    } else if crate::resolve::which_on_path("podman") {
        Some("podman")
    } else if crate::resolve::which_on_path("docker") {
        Some("docker")
    } else {
        None
    }?;

    Some(BackendContext {
        kind,
        rootless: detect_backend_rootless(kind),
    })
}

fn detect_backend_rootless(kind: &str) -> Option<bool> {
    match kind {
        "podman" => {
            let output = run_capture(
                Command::new("podman").args(["info", "--format", "{{.Host.Security.Rootless}}"]),
            )
            .ok()?;
            match output.trim() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            }
        }
        "docker" => {
            let output = run_capture(
                Command::new("docker").args(["info", "--format", "{{.SecurityOptions}}"]),
            )
            .ok()?;
            Some(output.contains("rootless"))
        }
        _ => None,
    }
}

fn run_capture(command: &mut Command) -> Result<String, String> {
    let output = command.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("command exited with status {}", output.status))
        } else {
            Err(stderr)
        }
    }
}

fn resolve_rootless_for_interactive(
    theme: &ColorfulTheme,
    backend: BackendContext,
) -> Result<bool, SboxError> {
    match backend.rootless {
        Some(rootless) => Ok(rootless),
        None => Confirm::with_theme(theme)
            .with_prompt(format!(
                "Could not detect whether {} is running in rootless mode. Is rootless mode enabled?",
                backend.kind
            ))
            .default(backend.kind == "podman")
            .interact()
            .map_err(|_| SboxError::CurrentDirectory {
                source: std::io::Error::other("prompt cancelled"),
            }),
    }
}

fn backend_runtime_lines(kind: &str, rootless: bool) -> (&'static str, &'static str) {
    match (kind, rootless) {
        ("podman", true) => ("  backend: podman", "  rootless: true"),
        ("podman", false) => ("  backend: podman", "  rootless: false"),
        ("docker", true) => ("  backend: docker", "  rootless: true"),
        ("docker", false) => ("  backend: docker", "  rootless: false"),
        _ => ("  # backend: auto-detected", "  # rootless: auto-detected"),
    }
}

fn project_is_containerized(project: &ProjectContext) -> bool {
    project.container_definition.is_some() || project.compose.is_some()
}

fn detect_compose_info(cwd: &Path) -> Option<ComposeInfo> {
    for name in &[
        "compose.yaml",
        "compose.yml",
        "docker-compose.yml",
        "docker-compose.yaml",
        "podman-compose.yml",
        "podman-compose.yaml",
    ] {
        let path = cwd.join(name);
        if !path.exists() {
            continue;
        }

        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };

        let mut candidates: Vec<(String, String, Vec<String>, Vec<String>)> = Vec::new();
        let mut all_services: Vec<String> = Vec::new();
        let mut current_service = String::new();
        let mut in_services = false;
        let mut service_indent: Option<usize> = None;
        let mut in_ports = false;
        let mut in_environment = false;

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            if trimmed == "services:" {
                in_services = true;
                service_indent = None;
                continue;
            }
            if !line.starts_with(' ') && !line.starts_with('\t') {
                in_services = false;
                continue;
            }

            if !in_services {
                continue;
            }

            let indent = line.len() - line.trim_start().len();
            let svc_indent = *service_indent.get_or_insert(indent);

            if indent == svc_indent && trimmed.ends_with(':') && !trimmed.contains(' ') {
                current_service = trimmed.trim_end_matches(':').to_string();
                all_services.push(current_service.clone());
                candidates.push((
                    current_service.clone(),
                    String::new(),
                    Vec::new(),
                    Vec::new(),
                ));
                in_ports = false;
                in_environment = false;
                continue;
            }

            if indent > svc_indent {
                if trimmed == "ports:" {
                    in_ports = true;
                    in_environment = false;
                    continue;
                }
                if trimmed == "environment:" {
                    in_ports = false;
                    in_environment = true;
                    continue;
                }

                if let Some(rest) = trimmed.strip_prefix("image:") {
                    let img = rest.trim().trim_matches('"').trim_matches('\'');
                    if !img.is_empty() {
                        if let Some(c) = candidates
                            .iter_mut()
                            .find(|(svc, _, _, _)| svc == &current_service)
                        {
                            c.1 = img.to_string();
                        }
                    }
                    in_ports = false;
                    in_environment = false;
                } else if in_ports && trimmed.starts_with("- ") {
                    let port = trimmed
                        .strip_prefix("- ")
                        .unwrap()
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'');
                    if let Some(c) = candidates
                        .iter_mut()
                        .find(|(svc, _, _, _)| svc == &current_service)
                    {
                        c.2.push(port.to_string());
                    }
                } else if in_environment && trimmed.starts_with("- ") {
                    let env = trimmed
                        .strip_prefix("- ")
                        .unwrap()
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'');
                    if let Some(c) = candidates
                        .iter_mut()
                        .find(|(svc, _, _, _)| svc == &current_service)
                    {
                        c.3.push(env.to_string());
                    }
                } else if in_environment && trimmed.contains(':') {
                    // key: value format
                    if let Some(c) = candidates
                        .iter_mut()
                        .find(|(svc, _, _, _)| svc == &current_service)
                    {
                        c.3.push(trimmed.to_string());
                    }
                } else if indent <= svc_indent + (svc_indent.max(2)) && !trimmed.starts_with("- ") {
                    // Probably another key at service level
                    in_ports = false;
                    in_environment = false;
                }
            }
        }

        if candidates.is_empty() {
            continue;
        }

        let mut primary = &candidates[0];
        let mut found_preferred = false;
        for &preferred in APP_SERVICE_NAMES {
            if let Some(c) = candidates.iter().find(|(svc, _, _, _)| svc == preferred) {
                primary = c;
                found_preferred = true;
                break;
            }
        }

        if !found_preferred {
            for c in &candidates {
                if !c.1.is_empty() {
                    let img_lower = c.1.to_lowercase();
                    let is_sidecar = COMPOSE_SIDECAR_PREFIXES
                        .iter()
                        .any(|p| img_lower.starts_with(p));
                    if !is_sidecar {
                        primary = c;
                        break;
                    }
                }
            }
        }

        let sidecars: Vec<String> = all_services
            .into_iter()
            .filter(|s| s != &primary.0)
            .collect();

        return Some(ComposeInfo {
            image: if primary.1.is_empty() {
                None
            } else {
                Some(primary.1.clone())
            },
            ports: primary.2.clone(),
            env: primary.3.clone(),
            sidecar_services: sidecars,
            compose_file: name.to_string(),
        });
    }
    None
}

fn execute_interactive_simple(
    theme: &ColorfulTheme,
    project: &ProjectContext,
) -> Result<String, SboxError> {
    // ── Package manager ───────────────────────────────────────────────────────
    let pm_name = if let Some(pm) = project.package_manager.detected {
        println!("Using detected package manager: {pm}");
        Some(pm)
    } else {
        let mut pm_choices = vec!["skip package_manager config"];
        pm_choices.extend(project.package_manager.choices.iter().copied());

        let default_idx = usize::from(!project_is_containerized(project));
        let pm_idx = Select::with_theme(theme)
            .with_prompt("Package manager")
            .items(&pm_choices)
            .default(default_idx)
            .interact()
            .map_err(|_| SboxError::CurrentDirectory {
                source: std::io::Error::other("prompt cancelled"),
            })?;

        if pm_idx == 0 {
            None
        } else {
            Some(pm_choices[pm_idx])
        }
    };

    let stock_image = [
        ("npm", "node:22-bookworm-slim"),
        ("yarn", "node:22-bookworm-slim"),
        ("pnpm", "node:22-bookworm-slim"),
        ("bun", "oven/bun:1"),
        ("uv", "ghcr.io/astral-sh/uv:python3.13-bookworm-slim"),
        ("pip", "python:3.13-slim"),
        ("poetry", "python:3.13-slim"),
        ("cargo", "rust:1-bookworm"),
        ("go", "golang:1.23-bookworm"),
    ]
    .into_iter()
    .find_map(|(name, image)| (Some(name) == pm_name).then_some(image))
    .unwrap_or("ubuntu:24.04");

    // ── Image — prefer existing Docker infrastructure over stock public images ─
    let mut image_line: String =
        if let Some(ref container_definition) = project.container_definition {
            println!(
                "Using detected container definition: {}",
                container_definition.path
            );
            format!("  build: {}", container_definition.path)
        } else if let Some(ref compose) = project.compose {
            if let Some(ref compose_image) = compose.image {
                println!("Using image from compose: {compose_image}");
                format!("  ref: {compose_image}")
            } else {
                let img = prompt_image(theme, stock_image)?;
                format!("  ref: {img}")
            }
        } else {
            let img = prompt_image(theme, stock_image)?;
            format!("  ref: {img}")
        };

    if let Some(reference) = image_line.strip_prefix("  ref: ") {
        if let Some(digest) = prompt_digest_on_failure(theme, reference.trim())? {
            image_line = format!("{image_line}\n  digest: {digest}");
        }
    }

    // ── Backend ───────────────────────────────────────────────────────────────
    let runtime_block = match project.backend {
        Some(backend) => {
            println!("Using detected backend: {}", backend.kind);
            let rootless = resolve_rootless_for_interactive(theme, backend)?;
            println!("Using {} rootless mode: {}", backend.kind, rootless);
            match (backend.kind, rootless) {
                ("podman", true) => "runtime:\n  backend: podman\n  rootless: true\n",
                ("podman", false) => "runtime:\n  backend: podman\n  rootless: false\n",
                ("docker", true) => "runtime:\n  backend: docker\n  rootless: true\n",
                ("docker", false) => "runtime:\n  backend: docker\n  rootless: false\n",
                _ => "",
            }
        }
        _ => {
            let backend_idx = Select::with_theme(theme)
                .with_prompt("Container backend")
                .items(&["auto (detect podman or docker)", "podman", "docker"])
                .default(0)
                .interact()
                .map_err(|_| SboxError::CurrentDirectory {
                    source: std::io::Error::other("prompt cancelled"),
                })?;
            match backend_idx {
                1 => {
                    let rootless = Confirm::with_theme(theme)
                        .with_prompt("Is Podman running in rootless mode?")
                        .default(true)
                        .interact()
                        .map_err(|_| SboxError::CurrentDirectory {
                            source: std::io::Error::other("prompt cancelled"),
                        })?;
                    if rootless {
                        "runtime:\n  backend: podman\n  rootless: true\n"
                    } else {
                        "runtime:\n  backend: podman\n  rootless: false\n"
                    }
                }
                2 => {
                    let rootless = Confirm::with_theme(theme)
                        .with_prompt("Is Docker running in rootless mode?")
                        .default(false)
                        .interact()
                        .map_err(|_| SboxError::CurrentDirectory {
                            source: std::io::Error::other("prompt cancelled"),
                        })?;
                    if rootless {
                        "runtime:\n  backend: docker\n  rootless: true\n"
                    } else {
                        "runtime:\n  backend: docker\n  rootless: false\n"
                    }
                }
                _ => "",
            }
        }
    };

    let mut config = if let Some(pm_name) = pm_name {
        let preset = match pm_name {
            "npm" | "yarn" | "pnpm" | "bun" => "node",
            "uv" | "pip" | "poetry" => "python",
            "cargo" => "rust",
            "go" => "go",
            _ => "generic",
        };

        let mut config =
            render_template_with_image_line(preset, &image_line).expect("known preset");

        config = config.replace(
            &format!(
                "name: {preset_pm}",
                preset_pm = match preset {
                    "node" => "npm",
                    "python" => "uv",
                    "rust" => "cargo",
                    "go" => "go",
                    _ => pm_name,
                }
            ),
            &format!("name: {pm_name}"),
        );
        config
    } else {
        render_generic_template_with_image_line(&image_line)?
    };

    // Prepend runtime block if the user selected one explicitly.
    if !runtime_block.is_empty() {
        let backend_kind = if runtime_block.contains("backend: docker") {
            "docker"
        } else {
            "podman"
        };
        let rootless = if runtime_block.contains("rootless: true") {
            Some(true)
        } else if runtime_block.contains("rootless: false") {
            Some(false)
        } else {
            None
        };
        config = apply_runtime_backend(config, backend_kind, rootless);
    }

    // ── Compose integration ───────────────────────────────────────────────────
    if let Some(compose) = &project.compose {
        let import_compose = Confirm::with_theme(theme)
            .with_prompt(format!(
                "Found `{}`. Import ports and env vars into sbox.yaml?",
                compose.compose_file
            ))
            .default(true)
            .interact()
            .map_err(|_| SboxError::CurrentDirectory {
                source: std::io::Error::other("prompt cancelled"),
            })?;

        if import_compose {
            if !compose.ports.is_empty() {
                let ports_yaml = compose
                    .ports
                    .iter()
                    .map(|p| format!("    - \"{p}\""))
                    .collect::<Vec<_>>()
                    .join("\n");
                config = config.replace("#   ports:\n#     - \"8000:8000\"", &format!("  ports:\n{ports_yaml}"));
            }

            if !compose.env.is_empty() {
                let env_yaml = compose
                    .env
                    .iter()
                    .map(|e| format!("    {e}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                config = config.replace("#   set:\n#     KEY: value", &format!("  set:\n{env_yaml}"));
            }

            if !compose.sidecar_services.is_empty() {
                config =
                    apply_runtime_compose(config, &compose.compose_file, &compose.sidecar_services);
            }
        }
    }

    Ok(config)
}

fn render_generic_template_with_image_line(image_line: &str) -> Result<String, SboxError> {
    let config = render_template("generic")?;
    Ok(config.replacen(
        "image:\n  ref: ubuntu:24.04",
        &format!("image:\n{image_line}"),
        1,
    ))
}

fn maybe_append_command_aliases(
    theme: &ColorfulTheme,
    config: String,
) -> Result<String, SboxError> {
    let add_aliases = Confirm::with_theme(theme)
        .with_prompt("Configure command aliases?")
        .default(false)
        .interact()
        .map_err(|_| SboxError::CurrentDirectory {
            source: std::io::Error::other("prompt cancelled"),
        })?;

    if !add_aliases {
        return Ok(config);
    }

    let aliases = prompt_command_aliases(theme)?;
    if aliases.is_empty() {
        return Ok(config);
    }

    Ok(insert_command_aliases(config, &aliases))
}

fn prompt_command_aliases(theme: &ColorfulTheme) -> Result<Vec<CommandAliasPrompt>, SboxError> {
    let mut aliases = Vec::new();

    loop {
        let add_alias = Confirm::with_theme(theme)
            .with_prompt("Add a command alias?")
            .default(aliases.is_empty())
            .interact()
            .map_err(|_| SboxError::CurrentDirectory {
                source: std::io::Error::other("prompt cancelled"),
            })?;

        if !add_alias {
            break;
        }

        let name: String = Input::with_theme(theme)
            .with_prompt("Alias name")
            .interact_text()
            .map_err(|_| SboxError::CurrentDirectory {
                source: std::io::Error::other("prompt cancelled"),
            })?;

        let run_line: String = Input::with_theme(theme)
            .with_prompt("Command to run (space-separated)")
            .interact_text()
            .map_err(|_| SboxError::CurrentDirectory {
                source: std::io::Error::other("prompt cancelled"),
            })?;

        let run = run_line
            .split_whitespace()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        if run.is_empty() {
            continue;
        }

        let description: String = Input::with_theme(theme)
            .with_prompt("Description (optional)")
            .allow_empty(true)
            .interact_text()
            .map_err(|_| SboxError::CurrentDirectory {
                source: std::io::Error::other("prompt cancelled"),
            })?;

        let default_profile_idx = match recommended_alias_profile(&run) {
            Some("default") => 1,
            Some("host") => 2,
            _ => 0,
        };

        if let Some(profile) = recommended_alias_profile(&run) {
            println!("Using detected profile recommendation: {profile}");
        }

        let profile_idx = Select::with_theme(theme)
            .with_prompt("Profile for this alias")
            .items(&["skip", "default", "host"])
            .default(default_profile_idx)
            .interact()
            .map_err(|_| SboxError::CurrentDirectory {
                source: std::io::Error::other("prompt cancelled"),
            })?;

        let profile = match profile_idx {
            1 => Some("default".to_string()),
            2 => Some("host".to_string()),
            _ => None,
        };

        aliases.push(CommandAliasPrompt {
            name: name.trim().to_string(),
            run,
            profile,
            description: (!description.trim().is_empty()).then_some(description.trim().to_string()),
        });
    }

    Ok(aliases)
}

fn recommended_alias_profile(run: &[String]) -> Option<&'static str> {
    match run.first().map(String::as_str) {
        Some("docker" | "docker-compose" | "podman" | "podman-compose") => Some("host"),
        _ => None,
    }
}

fn render_command_alias_yaml(alias: &CommandAliasPrompt) -> String {
    let run = alias
        .run
        .iter()
        .map(|part| format!("\"{}\"", part.replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ");

    let mut yaml = format!("  {}:\n    run: [{}]", alias.name, run);
    if let Some(profile) = &alias.profile {
        yaml.push_str(&format!("\n    profile: {profile}"));
    }
    if let Some(description) = &alias.description {
        yaml.push_str(&format!(
            "\n    description: \"{}\"",
            description.replace('"', "\\\"")
        ));
    }
    yaml
}

fn insert_command_aliases(config: String, aliases: &[CommandAliasPrompt]) -> String {
    let commands_yaml = aliases
        .iter()
        .map(render_command_alias_yaml)
        .collect::<Vec<_>>()
        .join("\n");

    let example_block = "# commands:  # Example command alias.\n#   build:\n#     run: [\"npm\", \"run\", \"build\"]\n#     profile: default\n#     description: \"Build project\"";

    if config.contains(example_block) {
        config.replacen(
            example_block,
            &format!("commands:\n{commands_yaml}"),
            1,
        )
    } else {
        config.replacen(
            "# ── Custom commands",
            &format!("commands:\n{commands_yaml}\n\n# ── Custom commands"),
            1,
        )
    }
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

fn prompt_digest_on_failure(
    theme: &ColorfulTheme,
    reference: &str,
) -> Result<Option<String>, SboxError> {
    let mut attempts: u8 = 0;
    loop {
        if let Some(digest) = try_fetch_image_digest(reference) {
            return Ok(Some(digest));
        }

        let idx = Select::with_theme(theme)
            .with_prompt(format!(
                "Could not fetch image digest for `{reference}`. What do you want to do?"
            ))
            .items(&[
                "retry digest fetch",
                "enter digest manually",
                "continue without pinning",
            ])
            .default(0)
            .interact()
            .map_err(|_| SboxError::CurrentDirectory {
                source: std::io::Error::other("prompt cancelled"),
            })?;

        match idx {
            0 => {
                attempts = attempts.saturating_add(1);
                if attempts >= 5 {
                    let cont = Confirm::with_theme(theme)
                        .with_prompt(
                            "Digest fetch keeps failing. Continue without pinning the image?",
                        )
                        .default(false)
                        .interact()
                        .map_err(|_| SboxError::CurrentDirectory {
                            source: std::io::Error::other("prompt cancelled"),
                        })?;
                    if cont {
                        return Ok(None);
                    }
                }
                continue;
            }
            1 => {
                let input: String = Input::with_theme(theme)
                    .with_prompt("Image digest (sha256:...)")
                    .allow_empty(true)
                    .interact_text()
                    .map_err(|_| SboxError::CurrentDirectory {
                        source: std::io::Error::other("prompt cancelled"),
                    })?;
                let digest = input.trim();
                if digest.is_empty() {
                    continue;
                }
                if digest.starts_with("sha256:") {
                    return Ok(Some(digest.to_string()));
                }
                let cont = Confirm::with_theme(theme)
                    .with_prompt("Digest does not start with `sha256:`. Use it anyway?")
                    .default(false)
                    .interact()
                    .map_err(|_| SboxError::CurrentDirectory {
                        source: std::io::Error::other("prompt cancelled"),
                    })?;
                if cont {
                    return Ok(Some(digest.to_string()));
                }
            }
            _ => {
                let cont = Confirm::with_theme(theme)
                    .with_prompt("Continue without pinning the image digest?")
                    .default(false)
                    .interact()
                    .map_err(|_| SboxError::CurrentDirectory {
                        source: std::io::Error::other("prompt cancelled"),
                    })?;
                if cont {
                    return Ok(None);
                }
            }
        }
    }
}

fn try_fetch_image_digest(reference: &str) -> Option<String> {
    if reference.contains("@sha256:") {
        return None;
    }

    let output = Command::new("skopeo")
        .args([
            "inspect",
            &format!("docker://{reference}"),
            "--format",
            "{{.Digest}}",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let digest = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if digest.starts_with("sha256:") {
        Some(digest)
    } else {
        None
    }
}

fn execute_interactive_advanced(
    theme: &ColorfulTheme,
    project: &ProjectContext,
) -> Result<String, SboxError> {
    // ── Backend ───────────────────────────────────────────────────────────────
    let (backend_line, rootless_line) = match project.backend {
        Some(backend) => {
            println!("Using detected backend: {}", backend.kind);
            let rootless = resolve_rootless_for_interactive(theme, backend)?;
            println!("Using {} rootless mode: {}", backend.kind, rootless);
            backend_runtime_lines(backend.kind, rootless)
        }
        _ => {
            let backend_idx = Select::with_theme(theme)
                .with_prompt("Container backend")
                .items(&["auto (detect podman or docker)", "podman", "docker"])
                .default(0)
                .interact()
                .map_err(|_| SboxError::CurrentDirectory {
                    source: std::io::Error::other("prompt cancelled"),
                })?;
            match backend_idx {
                1 => {
                    let rootless = Confirm::with_theme(theme)
                        .with_prompt("Is Podman running in rootless mode?")
                        .default(true)
                        .interact()
                        .map_err(|_| SboxError::CurrentDirectory {
                            source: std::io::Error::other("prompt cancelled"),
                        })?;
                    backend_runtime_lines("podman", rootless)
                }
                2 => {
                    let rootless = Confirm::with_theme(theme)
                        .with_prompt("Is Docker running in rootless mode?")
                        .default(false)
                        .interact()
                        .map_err(|_| SboxError::CurrentDirectory {
                            source: std::io::Error::other("prompt cancelled"),
                        })?;
                    backend_runtime_lines("docker", rootless)
                }
                _ => ("  # backend: auto-detected", "  # rootless: auto-detected"),
            }
        }
    };

    // ── Preset / image ────────────────────────────────────────────────────────
    // Build the ecosystem list, prepending detected local infrastructure.
    let mut image_choices: Vec<String> = Vec::new();
    if let Some(ref container_definition) = project.container_definition {
        image_choices.push(format!(
            "existing container definition ({})",
            container_definition.path
        ));
    }
    if let Some(ref compose) = project.compose {
        if let Some(ref img) = compose.image {
            image_choices.push(format!("image from compose ({img})"));
        }
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
    let has_compose_image = project
        .compose
        .as_ref()
        .and_then(|c| c.image.as_ref())
        .is_some();
    let offset = (project.container_definition.is_some() as usize) + (has_compose_image as usize);
    let ecosystem_names = ["node", "python", "rust", "go", "generic", "custom"];

    let (image_yaml, preset, default_writable_paths, default_dispatch) =
        if project.container_definition.is_some() && image_idx == 0 {
            let container_definition = project.container_definition.as_ref().unwrap();
            (
                format!("image:\n  build: {}", container_definition.path),
                "custom",
                vec![],
                String::new(),
            )
        } else if has_compose_image
            && image_idx == (project.container_definition.is_some() as usize)
        {
            let img = project.compose.as_ref().unwrap().image.as_ref().unwrap();
            let mut yaml = format!("image:\n  ref: {img}");
            if let Some(digest) = prompt_digest_on_failure(theme, img)? {
                yaml = format!("{yaml}\n  digest: {digest}");
            }
            (yaml, "custom", vec![], String::new())
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
            let mut yaml = format!("image:\n  ref: {img}");
            if let Some(digest) = prompt_digest_on_failure(theme, &img)? {
                yaml = format!("{yaml}\n  digest: {digest}");
            }
            (yaml, preset, writable, dispatch)
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

    let mut config = format!(
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
  # set:
  #   KEY: value
  # deny:
  #   - SECRET_TOKEN

profiles:
  default:
    mode: sandbox
    network: {network}
    writable: true
    no_new_privileges: true

{dispatch_section}
"
    );

    // ── Compose integration ───────────────────────────────────────────────────
    if let Some(compose) = &project.compose {
        let import_compose = Confirm::with_theme(theme)
            .with_prompt(format!(
                "Found `{}`. Import ports and env vars into sbox.yaml?",
                compose.compose_file
            ))
            .default(true)
            .interact()
            .map_err(|_| SboxError::CurrentDirectory {
                source: std::io::Error::other("prompt cancelled"),
            })?;

        if import_compose {
            if !compose.ports.is_empty() {
                let ports_yaml = compose
                    .ports
                    .iter()
                    .map(|p| format!("    - \"{p}\""))
                    .collect::<Vec<_>>()
                    .join("\n");
                config = config.replace(
                    "    no_new_privileges: true",
                    &format!("    no_new_privileges: true\n    ports:\n{ports_yaml}"),
                );
            }

            if !compose.env.is_empty() {
                let env_yaml = compose
                    .env
                    .iter()
                    .map(|e| format!("    {e}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                config = config.replace("  set: {}", &format!("  set:\n{env_yaml}"));
            }

            if !compose.sidecar_services.is_empty() {
                let services_yaml = compose
                    .sidecar_services
                    .iter()
                    .map(|s| format!("      - {s}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let compose_block = format!(
                    "  compose:\n    file: {}\n    services:\n{}\n",
                    compose.compose_file, services_yaml
                );
                config = config.replace("runtime:\n", &format!("runtime:\n{compose_block}"));
            }
        }
    }

    Ok(config)
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
        "node" => Ok(full_template(TemplateParams {
            pm_name: "npm",
            image_line: "  ref: node:22-bookworm-slim",
            exclude_paths: &[
                ".env",
                ".env.local",
                ".env.production",
                ".env.development",
                ".npmrc",
                ".netrc",
                "\".ssh/*\"",
                "\".aws/*\"",
            ],
            profile_comment: "\
# profiles:  # Example: dev server profile with published port 3000.\n\
#   serve:\n\
#     mode: sandbox\n\
#     network: on\n\
#     network_allow:\n\
#       - api.example.com\n\
#     ports:\n\
#       - \"3000:3000\"\n\
#     writable: false\n\
#     writable_paths:\n\
#       - node_modules",
            dispatch_comment: "\
# dispatch:  # Example: route dev-server commands to the `serve` profile.\n\
#   serve:\n\
#     match:\n\
#       - \"node*\"\n\
#       - \"npx*\"\n\
#     profile: serve",
        })),

        "python" | "uv" => Ok(full_template(TemplateParams {
            pm_name: "uv",
            image_line: "  ref: ghcr.io/astral-sh/uv:python3.13-bookworm-slim",
            exclude_paths: &[".env", ".env.local", ".netrc", "\".ssh/*\"", "\".aws/*\""],
            profile_comment: "\
# profiles:  # Example: app server profile with published port 8000.\n\
#   serve:\n\
#     mode: sandbox\n\
#     network: on\n\
#     network_allow:\n\
#       - api.example.com\n\
#     ports:\n\
#       - \"8000:8000\"\n\
#     writable: false\n\
#     writable_paths:\n\
#       - .venv",
            dispatch_comment: "\
# dispatch:  # Example: route `uv run ...` commands to the `serve` profile.\n\
#   serve:\n\
#     match:\n\
#       - \"uv run*\"\n\
#     profile: serve",
        })),

        "rust" => Ok(full_template(TemplateParams {
            pm_name: "cargo",
            image_line: "  ref: rust:1-bookworm",
            exclude_paths: &["\".ssh/*\"", "\".aws/*\""],
            profile_comment: "\
# profiles:  # Example: profile for running the compiled binary.\n\
#   run:\n\
#     mode: sandbox\n\
#     network: off\n\
#     writable: false\n\
#     writable_paths:\n\
#       - target",
            dispatch_comment: "\
# dispatch:  # Example: route `cargo run ...` to the `run` profile.\n\
#   run:\n\
#     match:\n\
#       - \"cargo run*\"\n\
#     profile: run",
        })),

        "go" => Ok(full_template(TemplateParams {
            pm_name: "go",
            image_line: "  ref: golang:1.23-bookworm",
            exclude_paths: &["\".ssh/*\"", "\".aws/*\""],
            profile_comment: "\
# profiles:  # Example: profile for running the compiled binary.\n\
#   run:\n\
#     mode: sandbox\n\
#     network: off\n\
#     writable: false",
            dispatch_comment: "\
# dispatch:  # Example: route `go run ...` to the `run` profile.\n\
#   run:\n\
#     match:\n\
#       - \"go run*\"\n\
#     profile: run",
        })),

        "generic" | "polyglot" => Ok("version: 1

# ── Runtime ───────────────────────────────────────────────────────────────────
# Leave `backend: podman` if you use rootless Podman. Change to `docker` when
# the project already uses Docker.
runtime:
  backend: podman
  rootless: true
  pull_policy: if-missing    # if-missing | always | never
  strict_security: false
  reuse_container: false
  require_pinned_image: false
  # compose:
  #   file: docker-compose.yml
  #   services:
  #     - db
  #     - redis

# ── Workspace ─────────────────────────────────────────────────────────────────
# `root` is the host directory mounted into the sandbox at `mount`.
workspace:
  root: .
  mount: /workspace
  writable: true
  # writable_paths:
  #   - dist
  exclude_paths:
    - \".ssh/*\"
    - \".aws/*\"
    # - .env
    # - .netrc

# ── Image ─────────────────────────────────────────────────────────────────────
# Use `ref:` for a published image. Switch to `build:` if the repo already has a
# Dockerfile or Containerfile that should define the sandbox image.
image:
  ref: ubuntu:24.04
  pull_policy: if-missing    # if-missing | always | never
  # digest: sha256:...
  # build: Dockerfile
  # preset: python

# ── Environment ───────────────────────────────────────────────────────────────
# Forward only the host variables you actually need.
environment:
  pass_through:
    - TERM
  # set:
  #   MY_VAR: value
  # deny:
  #   - SECRET_TOKEN

# ── Identity ──────────────────────────────────────────────────────────────────
identity:
  map_user: true
  # uid: 1000
  # gid: 1000

# ── Mounts ────────────────────────────────────────────────────────────────────
# mounts:
#   - type: bind
#     source: ./fixtures
#     target: /fixtures
#     read_only: true

# ── Caches ────────────────────────────────────────────────────────────────────
# caches:
#   - name: pip-cache
#     target: /root/.cache/pip
#     read_only: false

# ── Secrets ───────────────────────────────────────────────────────────────────
# secrets:
#   - name: pypi-token
#     source: /home/user/.config/pypi/token
#     target: /run/secrets/pypi-token
#     when_profiles:
#       - serve
#     deny_roles: [install]

# ── Profiles ──────────────────────────────────────────────────────────────────
# `default` handles commands that do not match a dispatch rule.
profiles:
  default:
    mode: sandbox
    network: off
    network_policy: dns      # dns | firewall
    writable: true
    no_new_privileges: true
    read_only_rootfs: false
    reuse_container: false
    shell: /bin/sh
    require_pinned_image: false
    require_lockfile: false
    capabilities:
      drop: [ALL]
      # add:
      #   - NET_BIND_SERVICE
    # network_allow:
    #   - api.example.com
    # writable_paths:
    #   - dist
    # ports:
    #   - \"8000:8000\"
    # compose:
    #   file: docker-compose.yml
    #   services:
    #     - db
    #     - redis

  host:
    mode: host
    network: on
    writable: true

# profiles:  # Example custom profile.
#   serve:
#     mode: sandbox
#     network: on
#     network_allow:
#       - api.example.com
#     writable: false
#     writable_paths:
#       - dist
#     ports:
#       - \"8000:8000\"

# ── Dispatch ──────────────────────────────────────────────────────────────────
# Routes commands to profiles. Patterns support * wildcards.
# dispatch:  # Example: route `python -m http.server ...` to the `serve` profile.
#   serve:
#     match:
#       - \"python -m http.server*\"
#     profile: serve

# ── Custom commands ───────────────────────────────────────────────────────────
# Define shortcuts such as `sbox build` or `sbox test`.
# commands:  # Example command alias.
#   build:
#     run: [\"cargo\", \"build\", \"--release\"]
#     profile: default        # optional
#     description: \"Build project\"
"
        .to_string()),

        other => Err(SboxError::UnknownPreset {
            name: other.to_string(),
        }),
    }
}

fn render_template_with_image_line(preset: &str, image_line: &str) -> Result<String, SboxError> {
    match preset {
        "node" => Ok(full_template(TemplateParams {
            pm_name: "npm",
            image_line,
            exclude_paths: &[
                ".env",
                ".env.local",
                ".env.production",
                ".env.development",
                ".npmrc",
                ".netrc",
                "\".ssh/*\"",
                "\".aws/*\"",
            ],
            profile_comment: "\
# profiles:  # Example: dev server profile with published port 3000.\n\
#   serve:\n\
#     mode: sandbox\n\
#     network: on\n\
#     network_allow:\n\
#       - api.example.com\n\
#     ports:\n\
#       - \"3000:3000\"\n\
#     writable: false\n\
#     writable_paths:\n\
#       - node_modules",
            dispatch_comment: "\
# dispatch:  # Example: route dev-server commands to the `serve` profile.\n\
#   serve:\n\
#     match:\n\
#       - \"node*\"\n\
#       - \"npx*\"\n\
#     profile: serve",
        })),

        "python" | "uv" => Ok(full_template(TemplateParams {
            pm_name: "uv",
            image_line,
            exclude_paths: &[".env", ".env.local", ".netrc", "\".ssh/*\"", "\".aws/*\""],
            profile_comment: "\
# profiles:  # Example: app server profile with published port 8000.\n\
#   serve:\n\
#     mode: sandbox\n\
#     network: on\n\
#     network_allow:\n\
#       - api.example.com\n\
#     ports:\n\
#       - \"8000:8000\"\n\
#     writable: false\n\
#     writable_paths:\n\
#       - .venv",
            dispatch_comment: "\
# dispatch:  # Example: route `uv run ...` commands to the `serve` profile.\n\
#   serve:\n\
#     match:\n\
#       - \"uv run*\"\n\
#     profile: serve",
        })),

        "rust" => Ok(full_template(TemplateParams {
            pm_name: "cargo",
            image_line,
            exclude_paths: &["\".ssh/*\"", "\".aws/*\""],
            profile_comment: "\
# profiles:  # Example: profile for running the compiled binary.\n\
#   run:\n\
#     mode: sandbox\n\
#     network: off\n\
#     writable: false\n\
#     writable_paths:\n\
#       - target",
            dispatch_comment: "\
# dispatch:  # Example: route `cargo run ...` to the `run` profile.\n\
#   run:\n\
#     match:\n\
#       - \"cargo run*\"\n\
#     profile: run",
        })),

        "go" => Ok(full_template(TemplateParams {
            pm_name: "go",
            image_line,
            exclude_paths: &["\".ssh/*\"", "\".aws/*\""],
            profile_comment: "\
# profiles:  # Example: profile for running the compiled binary.\n\
#   run:\n\
#     mode: sandbox\n\
#     network: off\n\
#     writable: false",
            dispatch_comment: "\
# dispatch:  # Example: route `go run ...` to the `run` profile.\n\
#   run:\n\
#     match:\n\
#       - \"go run*\"\n\
#     profile: run",
        })),

        other => render_template(other),
    }
}

struct TemplateParams<'a> {
    pm_name: &'static str,
    image_line: &'a str,
    exclude_paths: &'static [&'static str],
    profile_comment: &'static str,
    dispatch_comment: &'static str,
}

fn full_template(p: TemplateParams<'_>) -> String {
    let TemplateParams {
        pm_name,
        image_line,
        exclude_paths,
        profile_comment,
        dispatch_comment,
    } = p;
    let exclude = exclude_paths
        .iter()
        .map(|e| format!("    - {e}"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "version: 1

# ── Runtime ───────────────────────────────────────────────────────────────────
# Pin the backend here instead of relying on auto-detection. Podman is preferred
# when available, but switch to Docker if the project already uses it.
runtime:
  backend: podman
  rootless: true
  pull_policy: if-missing   # if-missing | always | never
  strict_security: false
  reuse_container: false
  require_pinned_image: false
  # compose:
  #   file: docker-compose.yml
  #   services:
  #     - db
  #     - redis

# ── Workspace ─────────────────────────────────────────────────────────────────
# The workspace is mounted read-only by default. Add paths below when a command
# needs to write artifacts such as lockfiles, dependency directories, or build output.
workspace:
  root: .
  mount: /workspace           # container path the workspace is mounted at
  writable: false             # read-only by default — safer for install sandboxes
  # writable_paths:
  #   - .venv
  exclude_paths:              # paths masked (not visible) inside the container
{exclude}

# ── Image ─────────────────────────────────────────────────────────────────────
# Pick either an image reference or a local build. Leave `ref:` as-is if the
# stock image is fine, or switch to `build:` if your repo already has a container file.
image:
{image_line}
  pull_policy: if-missing     # if-missing | always | never
  # digest: sha256:...
  # build: Dockerfile
  # preset: python

# ── Environment ───────────────────────────────────────────────────────────────
# Forward only the host variables you actually need.
environment:
  pass_through:
    - TERM                    # host env vars forwarded into the sandbox
  # set:
  #   KEY: value
  # deny:
  #   - SECRET_TOKEN

# ── Identity ──────────────────────────────────────────────────────────────────
identity:
  map_user: true
  # uid: 1000
  # gid: 1000

# ── Mounts ────────────────────────────────────────────────────────────────────
# mounts:
#   - type: bind
#     source: ./fixtures
#     target: /fixtures
#     read_only: true

# ── Caches ────────────────────────────────────────────────────────────────────
# caches:
#   - name: build-cache
#     target: /var/cache/build
#     read_only: false

# ── Secrets ───────────────────────────────────────────────────────────────────
# secrets:
#   - name: registry-token
#     source: /home/user/.config/registry/token
#     target: /run/secrets/registry-token
#     when_profiles:
#       - serve
#     deny_roles: [install]

# ── Package manager ───────────────────────────────────────────────────────────
# Automatically generates install/build/default profiles and dispatch rules.
package_manager:
  name: {pm_name}
  # install_writable:
  #   - .venv
  # build_writable:
  #   - dist
  # network_allow:
  #   - pypi.org
  # pre_run:
  #   - python -m pip --version

# ── Profiles ──────────────────────────────────────────────────────────────────
# package_manager: above generates install/build/default profiles automatically.
# Uncomment an extra profile only when you need behavior beyond what
# `package_manager:` generates automatically.
# Example extra profile:
{profile_comment}

# ── Dispatch ──────────────────────────────────────────────────────────────────
# Routes commands to profiles. Patterns use * wildcards.
# `package_manager:` already generates install/build dispatch rules.
# Example dispatch rule:
{dispatch_comment}

# ── Custom commands ───────────────────────────────────────────────────────────
# Define short aliases for common development tasks.
# commands:  # Example command alias.
#   build:
#     run: [\"npm\", \"run\", \"build\"]
#     profile: default
#     description: \"Build project\"
"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        apply_runtime_backend, detect_backend_context, detect_lockfile_preset,
        detect_package_manager_context, detect_package_manager_from_container_definition,
        insert_command_aliases, recommended_alias_profile, recommended_package_manager,
        render_generic_template_with_image_line, render_template, CommandAliasPrompt,
        ContainerDefinition, PackageManagerContext,
    };

    #[test]
    fn renders_node_template_with_package_manager() {
        let rendered = render_template("node").expect("node preset should exist");
        assert!(rendered.contains("ref: node:22-bookworm-slim"));
        assert!(rendered.contains("package_manager:"));
        assert!(rendered.contains("name: npm"));
        assert!(rendered.contains("# Example extra profile:"));
        assert!(rendered.contains("# commands:"));
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
    fn docker_backend_patch_disables_map_user() {
        let rendered = render_template("python").expect("python preset should exist");
        let rendered = apply_runtime_backend(rendered, "docker", Some(false));
        assert!(rendered.contains("backend: docker"));
        assert!(rendered.contains("rootless: false"));
        assert!(rendered.contains("map_user: false"));
    }

    #[test]
    fn renders_generic_template_with_profiles() {
        let rendered = render_template("generic").expect("generic preset should exist");
        assert!(rendered.contains("profiles:"));
        assert!(!rendered.contains("package_manager:"));
        assert!(rendered.contains("# commands:"));
    }

    #[test]
    fn detects_npm_from_package_lock_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package-lock.json"), "{}").unwrap();
        assert_eq!(detect_lockfile_preset(dir.path()), Some("npm"));
    }

    #[test]
    fn detects_yarn_from_yarn_lock() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("yarn.lock"), "").unwrap();
        assert_eq!(detect_lockfile_preset(dir.path()), Some("yarn"));
    }

    #[test]
    fn detects_uv_over_requirements_txt_when_both_present() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("uv.lock"), "").unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "").unwrap();
        // uv.lock appears before requirements.txt in the priority list
        assert_eq!(detect_lockfile_preset(dir.path()), Some("uv"));
    }

    #[test]
    fn detects_composer_from_composer_lock() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("composer.lock"), "{}").unwrap();
        assert_eq!(detect_lockfile_preset(dir.path()), Some("composer"));
    }

    #[test]
    fn detects_bundler_from_gemfile_lock() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Gemfile.lock"), "").unwrap();
        assert_eq!(detect_lockfile_preset(dir.path()), Some("bundler"));
    }

    #[test]
    fn returns_none_when_no_lockfile_found() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_lockfile_preset(dir.path()), None);
    }

    #[test]
    fn package_json_limits_choices_to_node_package_managers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{\"name\":\"demo\"}").unwrap();

        let context = detect_package_manager_context(dir.path());

        assert_eq!(context.detected, None);
        assert_eq!(context.choices, vec!["npm", "yarn", "pnpm", "bun"]);
        assert_eq!(recommended_package_manager(&context), Some("npm"));
    }

    #[test]
    fn package_json_package_manager_field_is_used_when_present() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            "{\"name\":\"demo\",\"packageManager\":\"pnpm@9.0.0\"}",
        )
        .unwrap();

        let context = detect_package_manager_context(dir.path());

        assert_eq!(context.detected, Some("pnpm"));
        assert_eq!(context.choices, vec!["pnpm"]);
    }

    #[test]
    fn pyproject_limits_choices_to_python_package_managers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname='demo'\n",
        )
        .unwrap();

        let context = detect_package_manager_context(dir.path());

        assert_eq!(context.detected, None);
        assert_eq!(context.choices, vec!["uv", "pip", "poetry"]);
        assert_eq!(recommended_package_manager(&context), Some("uv"));
    }

    #[test]
    fn backend_prefers_project_files_over_path_detection() {
        let container_definition = ContainerDefinition {
            path: "Containerfile".to_string(),
            backend_hint: "podman",
        };

        let backend = detect_backend_context(Some(&container_definition), None).unwrap();
        assert_eq!(backend.kind, "podman");
    }

    #[test]
    fn backend_patch_preserves_map_user_when_rootless_unknown() {
        let rendered = render_template("python").expect("python preset should exist");
        let rendered = apply_runtime_backend(rendered, "docker", None);
        assert!(rendered.contains("backend: docker"));
        assert!(rendered.contains("rootless: true"));
        assert!(rendered.contains("map_user: true"));
    }

    #[test]
    fn inserts_command_aliases_as_real_commands_block() {
        let rendered = render_template("python").expect("python preset should exist");
        let updated = insert_command_aliases(
            rendered,
            &[CommandAliasPrompt {
                name: "up".to_string(),
                run: vec![
                    "docker".to_string(),
                    "compose".to_string(),
                    "up".to_string(),
                    "--build".to_string(),
                ],
                profile: None,
                description: None,
            }],
        );

        assert!(updated.contains("commands:\n  up:\n    run: [\"docker\", \"compose\", \"up\", \"--build\"]"));
        assert!(!updated.contains("#   build:\n#     run: [\"npm\", \"run\", \"build\"]"));
    }

    #[test]
    fn recommends_host_profile_for_docker_aliases() {
        let run = vec![
            "docker".to_string(),
            "compose".to_string(),
            "up".to_string(),
            "--build".to_string(),
        ];
        assert_eq!(recommended_alias_profile(&run), Some("host"));
    }

    #[test]
    fn recommended_package_manager_uses_detected_value() {
        let context = PackageManagerContext {
            detected: Some("cargo"),
            choices: vec!["cargo"],
        };

        assert_eq!(recommended_package_manager(&context), Some("cargo"));
    }

    #[test]
    fn detects_package_manager_from_dockerfile_commands() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Dockerfile"),
            "FROM node:22\nRUN corepack enable pnpm && pnpm install --frozen-lockfile\n",
        )
        .unwrap();

        let container_definition = ContainerDefinition {
            path: "Dockerfile".to_string(),
            backend_hint: "docker",
        };

        assert_eq!(
            detect_package_manager_from_container_definition(dir.path(), &container_definition),
            Some("pnpm")
        );
    }

    #[test]
    fn generic_template_replaces_default_image_line() {
        let rendered =
            render_generic_template_with_image_line("  build: Dockerfile").expect("renders");

        assert!(rendered.contains("image:\n  build: Dockerfile"));
        assert!(!rendered.contains("ref: ubuntu:24.04"));
    }
}
