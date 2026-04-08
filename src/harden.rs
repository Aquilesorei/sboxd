use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use serde_yaml::{Mapping, Sequence, Value};

use crate::cli::{Cli, HardenCommand};
use crate::error::SboxError;

pub fn execute(cli: &Cli, command: &HardenCommand) -> Result<ExitCode, SboxError> {
    let cwd = cli
        .workspace
        .clone()
        .unwrap_or(std::env::current_dir().map_err(|source| SboxError::CurrentDirectory { source })?);
    let compose_path = resolve_compose_file(&cwd, command.compose_file.as_deref())?;
    let output_path = resolve_output_path(&compose_path, command.output.as_deref());
    let compose_text = fs::read_to_string(&compose_path).map_err(|source| SboxError::ConfigRead {
        path: compose_path.clone(),
        source,
    })?;
    let compose_yaml: Value =
        serde_yaml::from_str(&compose_text).map_err(|source| SboxError::ConfigParse {
            path: compose_path.clone(),
            source,
        })?;

    let report = analyze_project(&cwd, &compose_yaml);
    let override_yaml = generate_compose_override(&compose_yaml);

    if output_path.exists() && !command.write {
        return Err(SboxError::HardenOutputExists { path: output_path });
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|source| SboxError::HardenWrite {
            path: output_path.clone(),
            source,
        })?;
    }
    fs::write(&output_path, &override_yaml).map_err(|source| SboxError::HardenWrite {
        path: output_path.clone(),
        source,
    })?;

    println!("generated {}", output_path.display());
    print_report(&report);

    if command.diff {
        println!("\n# Generated Compose Override");
        println!("{override_yaml}");
    }

    if command.run {
        run_hardened_stack(&compose_path, &output_path)?;
    }

    Ok(ExitCode::SUCCESS)
}

fn resolve_compose_file(cwd: &Path, override_path: Option<&Path>) -> Result<PathBuf, SboxError> {
    if let Some(path) = override_path {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        return Ok(path);
    }

    for name in [
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
        "podman-compose.yml",
        "podman-compose.yaml",
    ] {
        let path = cwd.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }

    Err(SboxError::ConfigValidation {
        message: "no compose file found; expected docker-compose.yml, docker-compose.yaml, compose.yml, compose.yaml, podman-compose.yml, or podman-compose.yaml".to_string(),
    })
}

fn resolve_output_path(compose_path: &Path, override_path: Option<&Path>) -> PathBuf {
    match override_path {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => compose_path.parent().unwrap_or_else(|| Path::new(".")).join(path),
        None => compose_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("docker-compose.sbox.yml"),
    }
}

#[derive(Default)]
struct HardenReport {
    compose_findings: Vec<Finding>,
    dockerfile_findings: Vec<Finding>,
}

struct Finding {
    severity: Severity,
    summary: String,
    remediation: String,
}

#[derive(Clone, Copy)]
enum Severity {
    High,
    Medium,
    Low,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
        }
    }
}

fn analyze_project(cwd: &Path, compose_yaml: &Value) -> HardenReport {
    let mut report = HardenReport::default();
    if let Some(services) = compose_yaml
        .get("services")
        .and_then(Value::as_mapping)
    {
        for (name, service) in services {
            let service_name = name.as_str().unwrap_or("unknown");
            let Some(mapping) = service.as_mapping() else {
                continue;
            };
            let known_stateful = is_known_stateful_service(service_name, mapping);
            if mapping.get("privileged").and_then(Value::as_bool) == Some(true) {
                report.compose_findings.push(Finding {
                    severity: Severity::High,
                    summary: format!("service `{service_name}` uses `privileged: true`"),
                    remediation:
                        "Remove `privileged: true` and add back only the specific capabilities, devices, or mounts the service actually needs.".to_string(),
                });
            }
            if mapping
                .get("network_mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| mode == "host")
            {
                report.compose_findings.push(Finding {
                    severity: Severity::High,
                    summary: format!("service `{service_name}` uses `network_mode: host`"),
                    remediation:
                        "Prefer normal Compose networking and explicit `ports:` mappings so the service is not attached directly to the host network namespace.".to_string(),
                });
            }
            if !known_stateful && mapping.get("read_only").and_then(Value::as_bool) != Some(true) {
                report.compose_findings.push(Finding {
                    severity: Severity::Medium,
                    summary: format!("service `{service_name}` leaves the container root filesystem writable"),
                    remediation:
                        "Set `read_only: true` for services that do not need to write to the container rootfs, then add explicit writable volumes or `tmpfs:` only where the process actually needs write access.".to_string(),
                });
            }
            if let Some(volumes) = mapping.get("volumes").and_then(Value::as_sequence) {
                for volume in volumes {
                    if volume_is_writable(volume)
                        && !is_expected_stateful_volume(service_name, mapping, volume)
                    {
                        report.compose_findings.push(Finding {
                            severity: Severity::Medium,
                            summary: format!(
                                "service `{service_name}` mounts writable volume `{}`",
                                render_volume_for_report(volume)
                            ),
                            remediation:
                                "Make the mount read-only when possible, or narrow the writable target to the smallest path the service actually needs.".to_string(),
                        });
                    }
                }
            }
            if let Some(ports) = mapping.get("ports").and_then(Value::as_sequence) {
                for port in ports.iter().filter_map(Value::as_str) {
                    let colon_count = port.matches(':').count();
                    let binds_all_interfaces = port.starts_with("0.0.0.0:")
                        || (!port.starts_with("127.0.0.1:") && colon_count >= 1);
                    if binds_all_interfaces {
                        let severity = if known_stateful {
                            Severity::Low
                        } else {
                            Severity::Medium
                        };
                        report.compose_findings.push(Finding {
                            severity,
                            summary: format!(
                                "service `{service_name}` exposes broad port mapping `{port}`"
                            ),
                            remediation:
                                "Bind development ports to `127.0.0.1` unless the service truly needs to be reachable from other machines.".to_string(),
                        });
                    }
                }
            }
        }
    }

    let dockerfile_path = cwd.join("Dockerfile");
    if dockerfile_path.is_file() {
        if let Ok(content) = fs::read_to_string(&dockerfile_path) {
            let upper = content.to_uppercase();
            if !upper.lines().any(|line| line.trim_start().starts_with("USER ")) {
                report.dockerfile_findings.push(Finding {
                    severity: Severity::Medium,
                    summary: "Dockerfile does not set `USER` explicitly".to_string(),
                    remediation:
                        "Create a non-root runtime user and switch to it near the end of the Dockerfile.".to_string(),
                });
            }
            for (idx, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                let upper = trimmed.to_uppercase();
                if upper.starts_with("USER ") && trimmed[5..].trim() == "root" {
                    report.dockerfile_findings.push(Finding {
                        severity: Severity::Medium,
                        summary: format!("Dockerfile line {} runs as `root`", idx + 1),
                        remediation:
                            "Use root only for package installation steps, then switch back to a non-root user before the final image runs.".to_string(),
                    });
                }
            }
            if !upper.lines().any(|line| line.trim_start().starts_with("HEALTHCHECK ")) {
                report.dockerfile_findings.push(Finding {
                    severity: Severity::Low,
                    summary: "Dockerfile does not define `HEALTHCHECK`".to_string(),
                    remediation:
                        "Add a cheap healthcheck so Compose and orchestrators can distinguish a live process from a ready application.".to_string(),
                });
            }
        }
    }

    report
}

fn print_report(report: &HardenReport) {
    println!("harden report");
    println!("─────────────");
    if report.compose_findings.is_empty() {
        println!("compose: no high-signal findings");
    } else {
        println!("compose:");
        for finding in &report.compose_findings {
            println!("  - [{}] {}", finding.severity.label(), finding.summary);
            println!("    fix: {}", finding.remediation);
        }
    }
    if report.dockerfile_findings.is_empty() {
        println!("dockerfile: no high-signal findings");
    } else {
        println!("dockerfile:");
        for finding in &report.dockerfile_findings {
            println!("  - [{}] {}", finding.severity.label(), finding.summary);
            println!("    fix: {}", finding.remediation);
        }
    }
}

fn generate_compose_override(compose_yaml: &Value) -> String {
    let mut root = Mapping::new();
    let mut services_override = Mapping::new();

    if let Some(services) = compose_yaml
        .get("services")
        .and_then(Value::as_mapping)
    {
        for (name, service) in services {
            let Some(service_name) = name.as_str() else {
                continue;
            };
            let Some(service_mapping) = service.as_mapping() else {
                continue;
            };

            let mut override_mapping = Mapping::new();
            override_mapping.insert(Value::from("init"), Value::Bool(true));
            override_mapping.insert(
                Value::from("security_opt"),
                Value::Sequence(vec![Value::from("no-new-privileges:true")]),
            );

            if let Some(volumes) = service_mapping.get("volumes").and_then(Value::as_sequence) {
                let hardened = volumes
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|volume| volume.ends_with(":ro"))
                    .map(Value::from)
                    .collect::<Sequence>();
                if !hardened.is_empty() {
                    override_mapping.insert(Value::from("volumes"), Value::Sequence(hardened));
                }
            }

            services_override.insert(Value::from(service_name), Value::Mapping(override_mapping));
        }
    }

    root.insert(Value::from("services"), Value::Mapping(services_override));

    serde_yaml::to_string(&Value::Mapping(root)).expect("compose override should serialize")
}

fn run_hardened_stack(compose_path: &Path, override_path: &Path) -> Result<(), SboxError> {
    let backend = if compose_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("podman-compose"))
    {
        "podman"
    } else {
        "docker"
    };

    let args = [
        "compose",
        "-f",
        &compose_path.display().to_string(),
        "-f",
        &override_path.display().to_string(),
        "up",
        "--build",
    ];
    let status = Command::new(backend)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|source| SboxError::CommandSpawn {
            program: backend.to_string(),
            source,
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(SboxError::BackendCommandFailed {
            backend: backend.to_string(),
            command: format!(
                "{backend} compose -f {} -f {} up --build",
                compose_path.display(),
                override_path.display()
            ),
            status: status.code().unwrap_or(1),
        })
    }
}

fn volume_is_writable(volume: &Value) -> bool {
    match volume {
        Value::String(spec) => !spec.ends_with(":ro"),
        Value::Mapping(mapping) => mapping.get("read_only").and_then(Value::as_bool) != Some(true),
        _ => false,
    }
}

fn render_volume_for_report(volume: &Value) -> String {
    match volume {
        Value::String(spec) => spec.clone(),
        Value::Mapping(mapping) => {
            let target = mapping
                .get("target")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let source = mapping
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("<anonymous>");
            format!("{source}:{target}")
        }
        _ => "<unknown>".to_string(),
    }
}

fn is_known_stateful_service(service_name: &str, mapping: &Mapping) -> bool {
    let image = mapping
        .get("image")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let name = service_name.to_ascii_lowercase();
    ["postgres", "redis", "mysql", "mariadb", "mongodb"]
        .iter()
        .any(|needle| name.contains(needle) || image.contains(needle))
}

fn is_expected_stateful_volume(service_name: &str, mapping: &Mapping, volume: &Value) -> bool {
    let target = volume_target(volume);
    if !is_known_stateful_service(service_name, mapping) {
        return false;
    }
    matches!(
        target,
        Some("/var/lib/postgresql/data")
            | Some("/data")
            | Some("/var/lib/mysql")
            | Some("/var/lib/mariadb")
            | Some("/data/db")
    )
}

fn volume_target(volume: &Value) -> Option<&str> {
    match volume {
        Value::String(spec) => {
            let mut parts = spec.split(':');
            let _source = parts.next()?;
            parts.next()
        }
        Value::Mapping(mapping) => mapping.get("target").and_then(Value::as_str),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{analyze_project, generate_compose_override, resolve_output_path};
    use serde_yaml::Value;
    use std::path::{Path, PathBuf};

    #[test]
    fn generates_compose_override_with_init_and_no_new_privileges() {
        let compose: Value = serde_yaml::from_str(
            r#"
services:
  api:
    image: app:latest
  redis:
    image: redis:7
"#,
        )
        .unwrap();

        let rendered = generate_compose_override(&compose);
        assert!(rendered.contains("services:"));
        assert!(rendered.contains("init: true"));
        assert!(rendered.contains("no-new-privileges:true"));
    }

    #[test]
    fn dockerfile_report_flags_missing_user_and_healthcheck() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Dockerfile"),
            "FROM python:3.13-slim\nRUN python --version\n",
        )
        .unwrap();
        let compose: Value = serde_yaml::from_str("services: {}\n").unwrap();

        let report = analyze_project(dir.path(), &compose);
        assert!(report
            .dockerfile_findings
            .iter()
            .any(|w| w.summary.contains("USER")));
        assert!(report
            .dockerfile_findings
            .iter()
            .any(|w| w.summary.contains("HEALTHCHECK")));
    }

    #[test]
    fn compose_report_flags_writable_service_and_broad_network_exposure() {
        let dir = tempfile::tempdir().unwrap();
        let compose: Value = serde_yaml::from_str(
            r#"
services:
  api:
    image: app:latest
    ports:
      - "8000:8000"
    volumes:
      - .:/app
"#,
        )
        .unwrap();

        let report = analyze_project(dir.path(), &compose);
        assert!(report
            .compose_findings
            .iter()
            .any(|w| w.summary.contains("root filesystem writable")));
        assert!(report
            .compose_findings
            .iter()
            .any(|w| w.summary.contains("writable volume")));
        assert!(report
            .compose_findings
            .iter()
            .any(|w| w.summary.contains("broad port mapping")));
    }

    #[test]
    fn compose_report_ignores_expected_stateful_data_volumes() {
        let dir = tempfile::tempdir().unwrap();
        let compose: Value = serde_yaml::from_str(
            r#"
services:
  postgres:
    image: postgres:16
    volumes:
      - postgres_data:/var/lib/postgresql/data
  redis:
    image: redis:7
    volumes:
      - redis_data:/data
"#,
        )
        .unwrap();

        let report = analyze_project(dir.path(), &compose);
        assert!(!report
            .compose_findings
            .iter()
            .any(|w| w.summary.contains("postgres")));
        assert!(!report
            .compose_findings
            .iter()
            .any(|w| w.summary.contains("redis")));
    }

    #[test]
    fn stateful_service_port_exposure_is_low_severity() {
        let dir = tempfile::tempdir().unwrap();
        let compose: Value = serde_yaml::from_str(
            r#"
services:
  postgres:
    image: postgres:16
    ports:
      - "5433:5432"
  api:
    image: app:latest
    ports:
      - "8000:8000"
"#,
        )
        .unwrap();

        let report = analyze_project(dir.path(), &compose);
        let postgres = report
            .compose_findings
            .iter()
            .find(|w| w.summary.contains("postgres") && w.summary.contains("broad port mapping"))
            .expect("postgres port finding");
        assert_eq!(postgres.severity.label(), "low");

        let api = report
            .compose_findings
            .iter()
            .find(|w| w.summary.contains("api") && w.summary.contains("broad port mapping"))
            .expect("api port finding");
        assert_eq!(api.severity.label(), "medium");
    }

    #[test]
    fn resolve_output_defaults_next_to_compose_file() {
        let path = resolve_output_path(Path::new("/tmp/docker-compose.yml"), None);
        assert_eq!(path, PathBuf::from("/tmp/docker-compose.sbox.yml"));
    }
}
