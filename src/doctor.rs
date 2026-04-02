use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use crate::cli::{Cli, CliBackendKind, DoctorCommand};
use crate::config::{LoadOptions, load_config};
use crate::resolve::{ResolutionTarget, ResolvedImageSource, resolve_execution_plan};

pub fn execute(cli: &Cli, command: &DoctorCommand) -> Result<ExitCode, crate::error::SboxError> {
    let mut checks = Vec::new();

    let loaded = match load_config(&LoadOptions {
        workspace: cli.workspace.clone(),
        config: cli.config.clone(),
    }) {
        Ok(loaded) => {
            checks.push(CheckResult::pass(
                "config",
                format!("loaded {}", loaded.config_path.display()),
            ));
            Some(loaded)
        }
        Err(error) => {
            checks.push(CheckResult::fail("config", error.to_string()));
            None
        }
    };

    let backend = resolve_backend(cli, loaded.as_ref());
    if let Some(loaded) = loaded.as_ref() {
        checks.extend(risky_config_warnings(&loaded.config));
        checks.extend(workspace_state_warnings(loaded));
    }
    match backend {
        Backend::Podman => run_podman_checks(cli, loaded.as_ref(), &mut checks),
        Backend::Docker => checks.push(CheckResult::warn(
            "backend",
            "docker doctor checks are not implemented yet".to_string(),
        )),
    }

    print_report(&checks);
    Ok(determine_exit_code(&checks, command.strict))
}

#[derive(Debug, Clone, Copy)]
enum Backend {
    Podman,
    Docker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckLevel {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
struct CheckResult {
    name: &'static str,
    level: CheckLevel,
    detail: String,
}

impl CheckResult {
    fn pass(name: &'static str, detail: String) -> Self {
        Self {
            name,
            level: CheckLevel::Pass,
            detail,
        }
    }

    fn warn(name: &'static str, detail: String) -> Self {
        Self {
            name,
            level: CheckLevel::Warn,
            detail,
        }
    }

    fn fail(name: &'static str, detail: String) -> Self {
        Self {
            name,
            level: CheckLevel::Fail,
            detail,
        }
    }
}

fn resolve_backend(cli: &Cli, loaded: Option<&crate::config::LoadedConfig>) -> Backend {
    match cli.backend {
        Some(CliBackendKind::Docker) => Backend::Docker,
        Some(CliBackendKind::Podman) => Backend::Podman,
        None => match loaded
            .and_then(|loaded| loaded.config.runtime.as_ref())
            .and_then(|runtime| runtime.backend.as_ref())
        {
            Some(crate::config::BackendKind::Docker) => Backend::Docker,
            _ => Backend::Podman,
        },
    }
}

fn run_podman_checks(
    cli: &Cli,
    loaded: Option<&crate::config::LoadedConfig>,
    checks: &mut Vec<CheckResult>,
) {
    let installed = run_capture(Command::new("podman").arg("--version"));
    if let Err(detail) = installed {
        checks.push(CheckResult::fail("backend", detail));
        return;
    }
    checks.push(CheckResult::pass(
        "backend",
        "podman is installed".to_string(),
    ));

    match run_capture(Command::new("podman").args([
        "info",
        "--format",
        "{{.Host.Security.Rootless}}",
    ])) {
        Ok(output) if output.trim() == "true" => checks.push(CheckResult::pass(
            "rootless",
            "rootless mode is available".to_string(),
        )),
        Ok(output) => checks.push(CheckResult::fail(
            "rootless",
            format!("podman reported rootless={}", output.trim()),
        )),
        Err(detail) => checks.push(CheckResult::fail("rootless", detail)),
    }

    if let Some(loaded) = loaded {
        checks.push(signature_verification_check(&loaded.config));
        match mount_check_request(cli, loaded) {
            Ok(request) => match run_status(podman_mount_probe(&request)) {
                Ok(()) => checks.push(CheckResult::pass(
                    "workspace-mount",
                    format!(
                        "workspace mounted at {} with cwd {}",
                        request.workspace_mount, request.sandbox_cwd
                    ),
                )),
                Err(detail) => checks.push(CheckResult::fail("workspace-mount", detail)),
            },
            Err(detail) => checks.push(CheckResult::warn("workspace-mount", detail)),
        }
    }
}

fn signature_verification_check(config: &crate::config::model::Config) -> CheckResult {
    let requested = config
        .image
        .as_ref()
        .and_then(|image| image.verify_signature)
        .unwrap_or(false);

    match crate::backend::podman::inspect_signature_verification_support() {
        Ok(crate::backend::podman::SignatureVerificationSupport::Available { policy }) => {
            if requested {
                CheckResult::pass(
                    "signature-verify",
                    format!("requested and supported via {}", policy.display()),
                )
            } else {
                CheckResult::pass(
                    "signature-verify",
                    format!("available via {} (not requested by config)", policy.display()),
                )
            }
        }
        Ok(crate::backend::podman::SignatureVerificationSupport::Unavailable {
            policy,
            reason,
        }) => {
            let detail = match policy {
                Some(policy) => format!("{reason} ({})", policy.display()),
                None => reason,
            };
            if requested {
                CheckResult::fail("signature-verify", detail)
            } else {
                CheckResult::warn("signature-verify", format!("not currently usable: {detail}"))
            }
        }
        Err(error) => {
            if requested {
                CheckResult::fail("signature-verify", error.to_string())
            } else {
                CheckResult::warn("signature-verify", error.to_string())
            }
        }
    }
}

struct MountCheckRequest {
    image: String,
    workspace_root: String,
    workspace_mount: String,
    sandbox_cwd: String,
    userns_keep_id: bool,
}

fn mount_check_request(
    cli: &Cli,
    loaded: &crate::config::LoadedConfig,
) -> Result<MountCheckRequest, String> {
    let plan = resolve_execution_plan(
        cli,
        loaded,
        ResolutionTarget::Plan,
        &["__doctor__".to_string()],
    )
    .map_err(|error| format!("unable to resolve workspace mount test: {error}"))?;

    let image = match &plan.image.source {
        ResolvedImageSource::Reference(reference) => reference.clone(),
        ResolvedImageSource::Build { tag, .. } => tag.clone(),
        ResolvedImageSource::Preset(name) => {
            return Err(format!(
                "unsupported preset image source during doctor: preset:{name}"
            ));
        }
    };

    Ok(MountCheckRequest {
        image,
        workspace_root: plan.workspace.root.display().to_string(),
        workspace_mount: plan.workspace.mount,
        sandbox_cwd: plan.workspace.sandbox_cwd,
        userns_keep_id: matches!(plan.user, crate::resolve::ResolvedUser::KeepId),
    })
}

fn podman_mount_probe(request: &MountCheckRequest) -> Command {
    let mut command = Command::new("podman");
    command.arg("run");
    command.arg("--rm");
    if request.userns_keep_id {
        command.args(["--userns", "keep-id"]);
    }
    command.args([
        "--mount",
        &format!(
            "type=bind,src={},target={},relabel=private,readonly=false",
            request.workspace_root, request.workspace_mount
        ),
        "--workdir",
        &request.sandbox_cwd,
        "--entrypoint",
        "/bin/sh",
        &request.image,
        "-lc",
        "pwd >/dev/null && test -r .",
    ]);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::piped());
    command
}

fn print_report(checks: &[CheckResult]) {
    println!("sbox doctor");
    for check in checks {
        println!(
            "{} {:<16} {}",
            level_label(check.level),
            check.name,
            check.detail
        );
    }
}

fn risky_config_warnings(config: &crate::config::model::Config) -> Vec<CheckResult> {
    let mut checks = Vec::new();
    let sensitive_envs: Vec<String> = config
        .environment
        .as_ref()
        .map(|environment| {
            environment
                .pass_through
                .iter()
                .filter(|name| looks_like_sensitive_env(name))
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    if !sensitive_envs.is_empty() {
        checks.push(CheckResult::warn(
            "env-policy",
            format!(
                "sensitive host variables are passed through: {}",
                sensitive_envs.join(", ")
            ),
        ));
    }

    if !sensitive_envs.is_empty() {
        let risky_profiles: Vec<String> = config
            .profiles
            .iter()
            .filter(|(_, profile)| {
                matches!(profile.mode, crate::config::model::ExecutionMode::Sandbox)
                    && profile.network.as_deref().unwrap_or("off") != "off"
            })
            .map(|(name, _)| name.clone())
            .collect();

        if !risky_profiles.is_empty() {
            checks.push(CheckResult::warn(
                "install-policy",
                format!(
                    "network-enabled sandbox profiles can see sensitive pass-through vars: {}",
                    risky_profiles.join(", ")
                ),
            ));
        }
    }

    let risky_mounts: Vec<String> = config
        .mounts
        .iter()
        .filter_map(|mount| mount.source.as_deref())
        .filter(|source| looks_like_sensitive_mount(source))
        .map(|source| source.display().to_string())
        .collect();

    if !risky_mounts.is_empty() {
        checks.push(CheckResult::warn(
            "mount-policy",
            format!(
                "sensitive host paths are mounted explicitly: {}",
                risky_mounts.join(", ")
            ),
        ));
    }

    checks
}

fn workspace_state_warnings(loaded: &crate::config::LoadedConfig) -> Vec<CheckResult> {
    let mut checks = Vec::new();
    let risky_artifacts = scan_workspace_artifacts(&loaded.workspace_root);

    if !risky_artifacts.is_empty() {
        checks.push(CheckResult::warn(
            "workspace-state",
            format!(
                "host dependency artifacts exist in the workspace: {}",
                risky_artifacts.join(", ")
            ),
        ));
    }

    checks
}

fn scan_workspace_artifacts(workspace_root: &Path) -> Vec<String> {
    const RISKY_NAMES: &[&str] = &[
        ".venv",
        "node_modules",
        ".npmrc",
        ".yarnrc",
        ".yarnrc.yml",
        ".pnpm-store",
    ];
    const MAX_DEPTH: usize = 3;

    let mut findings = Vec::new();
    let mut stack = vec![(workspace_root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();

            if RISKY_NAMES.iter().any(|candidate| *candidate == name)
                && let Ok(relative) = path.strip_prefix(workspace_root)
            {
                findings.push(relative.display().to_string());
            }

            if depth < MAX_DEPTH
                && entry
                    .file_type()
                    .map(|file_type| file_type.is_dir())
                    .unwrap_or(false)
                && !name.starts_with('.')
            {
                stack.push((path, depth + 1));
            }
        }
    }

    findings.sort();
    findings
}

fn looks_like_sensitive_env(name: &str) -> bool {
    const EXACT: &[&str] = &[
        "SSH_AUTH_SOCK",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "NPM_TOKEN",
        "NODE_AUTH_TOKEN",
        "PYPI_TOKEN",
        "DOCKER_CONFIG",
        "KUBECONFIG",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "AZURE_CLIENT_SECRET",
        "AWS_SESSION_TOKEN",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_ACCESS_KEY_ID",
    ];
    const PREFIXES: &[&str] = &["AWS_", "GCP_", "GOOGLE_", "AZURE_", "CI_JOB_", "CLOUDSDK_"];

    EXACT.contains(&name) || PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

fn looks_like_sensitive_mount(source: &Path) -> bool {
    let source_string = source.to_string_lossy();
    if source_string == "~" || source_string.starts_with("~/") {
        return true;
    }

    if !source.is_absolute() {
        return false;
    }

    const EXACT_PATHS: &[&str] = &[
        "/var/run/docker.sock",
        "/run/docker.sock",
        "/var/run/podman/podman.sock",
        "/run/podman/podman.sock",
        "/home",
        "/root",
        "/Users",
    ];
    if EXACT_PATHS
        .iter()
        .any(|candidate| source == Path::new(candidate))
    {
        return true;
    }

    if let Some(home) = std::env::var_os("HOME") {
        let home = Path::new(&home);
        if source == home {
            return true;
        }

        for suffix in [
            ".ssh",
            ".aws",
            ".kube",
            ".config/gcloud",
            ".gnupg",
            ".git-credentials",
            ".npmrc",
            ".pypirc",
            ".netrc",
        ] {
            if source == home.join(suffix) {
                return true;
            }
        }
    }

    false
}

fn level_label(level: CheckLevel) -> &'static str {
    match level {
        CheckLevel::Pass => "PASS",
        CheckLevel::Warn => "WARN",
        CheckLevel::Fail => "FAIL",
    }
}

fn determine_exit_code(checks: &[CheckResult], strict: bool) -> ExitCode {
    if checks.iter().any(|check| check.level == CheckLevel::Fail) {
        return ExitCode::from(10);
    }

    if strict && checks.iter().any(|check| check.level == CheckLevel::Warn) {
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}

fn run_capture(command: &mut Command) -> Result<String, String> {
    let output = command.output().map_err(|source| source.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!(
                "command exited with status {}",
                output.status.code().unwrap_or(1)
            ))
        } else {
            Err(stderr)
        }
    }
}

fn run_status(mut command: Command) -> Result<(), String> {
    let output = command.output().map_err(|source| source.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!(
                "command exited with status {}",
                output.status.code().unwrap_or(1)
            ))
        } else {
            Err(stderr)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CheckLevel, CheckResult, determine_exit_code, risky_config_warnings,
        scan_workspace_artifacts, workspace_state_warnings,
    };
    use crate::config::LoadedConfig;
    use crate::config::model::{Config, EnvironmentConfig, MountConfig, MountType};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::process::ExitCode;

    #[test]
    fn doctor_returns_success_when_all_checks_pass() {
        let checks = vec![CheckResult::pass("config", "ok".into())];
        assert_eq!(determine_exit_code(&checks, false), ExitCode::SUCCESS);
    }

    #[test]
    fn doctor_returns_strict_warning_exit_code() {
        let checks = vec![CheckResult {
            name: "backend",
            level: CheckLevel::Warn,
            detail: "warn".into(),
        }];
        assert_eq!(determine_exit_code(&checks, true), ExitCode::from(1));
    }

    #[test]
    fn doctor_returns_failure_exit_code_when_any_check_fails() {
        let checks = vec![CheckResult::fail("backend", "missing".into())];
        assert_eq!(determine_exit_code(&checks, false), ExitCode::from(10));
    }

    #[test]
    fn doctor_warns_on_sensitive_pass_through_envs() {
        let config = Config {
            version: 1,
            runtime: None,
            workspace: None,
            identity: None,
            image: None,
            environment: Some(EnvironmentConfig {
                pass_through: vec!["AWS_SECRET_ACCESS_KEY".into()],
                set: BTreeMap::new(),
                deny: Vec::new(),
            }),
            mounts: Vec::new(),
            caches: Vec::new(),
            secrets: Vec::new(),
            profiles: Default::default(),
            dispatch: Default::default(),
        };

        let warnings = risky_config_warnings(&config);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.name == "env-policy" && warning.level == CheckLevel::Warn)
        );
    }

    #[test]
    fn doctor_warns_on_sensitive_mounts() {
        let config = Config {
            version: 1,
            runtime: None,
            workspace: None,
            identity: None,
            image: None,
            environment: None,
            mounts: vec![MountConfig {
                source: Some(PathBuf::from("/var/run/docker.sock")),
                target: Some("/run/docker.sock".into()),
                mount_type: MountType::Bind,
                read_only: Some(true),
                create: None,
            }],
            caches: Vec::new(),
            secrets: Vec::new(),
            profiles: Default::default(),
            dispatch: Default::default(),
        };

        let warnings = risky_config_warnings(&config);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.name == "mount-policy" && warning.level == CheckLevel::Warn)
        );
    }

    #[test]
    fn doctor_warns_when_sensitive_envs_meet_network_enabled_profiles() {
        let mut profiles = indexmap::IndexMap::new();
        profiles.insert(
            "install".to_string(),
            crate::config::model::ProfileConfig {
                mode: crate::config::model::ExecutionMode::Sandbox,
                image: None,
                network: Some("on".into()),
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
        let config = Config {
            version: 1,
            runtime: None,
            workspace: None,
            identity: None,
            image: None,
            environment: Some(EnvironmentConfig {
                pass_through: vec!["NPM_TOKEN".into()],
                set: BTreeMap::new(),
                deny: Vec::new(),
            }),
            mounts: Vec::new(),
            caches: Vec::new(),
            secrets: Vec::new(),
            profiles,
            dispatch: Default::default(),
        };

        let warnings = risky_config_warnings(&config);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.name == "install-policy")
        );
    }

    #[test]
    fn doctor_warns_on_workspace_dependency_artifacts() {
        let unique = format!(
            "sbox-doctor-workspace-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(root.join(".venv")).expect("fixture workspace should exist");
        std::fs::create_dir_all(root.join("examples/demo/node_modules"))
            .expect("nested dependency artifact should exist");

        let loaded = LoadedConfig {
            invocation_dir: root.clone(),
            workspace_root: root.clone(),
            config_path: root.join("sbox.yaml"),
            config: Config {
                version: 1,
                runtime: None,
                workspace: None,
                identity: None,
                image: None,
                environment: None,
                mounts: Vec::new(),
                caches: Vec::new(),
                secrets: Vec::new(),
                profiles: Default::default(),
                dispatch: Default::default(),
            },
        };

        let warnings = workspace_state_warnings(&loaded);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.name == "workspace-state")
        );
        assert!(warnings[0].detail.contains(".venv"));
        assert!(warnings[0].detail.contains("examples/demo/node_modules"));

        std::fs::remove_dir_all(root).expect("fixture workspace should be removed");
    }

    #[test]
    fn workspace_scan_finds_nested_dependency_artifacts() {
        let unique = format!(
            "sbox-doctor-scan-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(root.join("examples/npm-smoke/node_modules"))
            .expect("nested node_modules should exist");

        let findings = scan_workspace_artifacts(&root);
        assert!(
            findings
                .iter()
                .any(|path| path == "examples/npm-smoke/node_modules")
        );

        std::fs::remove_dir_all(root).expect("fixture workspace should be removed");
    }

    #[test]
    fn doctor_warning_exit_code_stays_nonfatal_when_signature_verification_is_not_requested() {
        let checks = vec![CheckResult::warn(
            "signature-verify",
            "not currently usable: skopeo is not installed".into(),
        )];
        assert_eq!(determine_exit_code(&checks, false), ExitCode::SUCCESS);
    }
}
