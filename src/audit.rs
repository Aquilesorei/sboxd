use std::process::{Command, ExitCode, Stdio};

use crate::cli::{AuditCommand, Cli};
use crate::config::{LoadOptions, load_config};
use crate::error::SboxError;
use crate::exec::status_to_exit_code;

/// `sbox audit` — scan the project's lockfile for known-malicious or vulnerable packages.
///
/// Delegates to the ecosystem's native audit tool and runs on the HOST (not in a sandbox)
/// so it can reach advisory databases. This is intentional — audit only reads the lockfile
/// and queries read-only advisory APIs; it does not execute package code.
pub fn execute(cli: &Cli, command: &AuditCommand) -> Result<ExitCode, SboxError> {
    let loaded = load_config(&LoadOptions {
        workspace: cli.workspace.clone(),
        config: cli.config.clone(),
    })?;

    let pm_name = loaded
        .config
        .package_manager
        .as_ref()
        .map(|pm| pm.name.as_str())
        .unwrap_or_else(|| detect_pm_from_workspace(&loaded.workspace_root));

    let (program, base_args, install_hint) = audit_command_for(pm_name);

    let mut child = Command::new(program);
    child.args(base_args);
    child.args(&command.extra_args);
    child.current_dir(&loaded.workspace_root);
    child.stdin(Stdio::inherit());
    child.stdout(Stdio::inherit());
    child.stderr(Stdio::inherit());

    match child.status() {
        Ok(status) => Ok(status_to_exit_code(status)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("sbox audit: `{program}` not found.");
            eprintln!("{install_hint}");
            Ok(ExitCode::from(127))
        }
        Err(source) => Err(SboxError::CommandSpawn {
            program: program.to_string(),
            source,
        }),
    }
}

/// Returns `(program, base_args, install_hint)` for the given package manager.
fn audit_command_for(pm_name: &str) -> (&'static str, &'static [&'static str], &'static str) {
    match pm_name {
        "npm" => (
            "npm",
            &["audit"] as &[&str],
            "npm is required. Install Node.js from https://nodejs.org",
        ),
        "yarn" => (
            "yarn",
            &["npm", "audit"],
            "yarn is required. Install from https://yarnpkg.com",
        ),
        "pnpm" => (
            "pnpm",
            &["audit"],
            "pnpm is required: npm install -g pnpm",
        ),
        "bun" => (
            // bun does not have a native audit command; delegate to npm audit which can read
            // package-lock.json or bun.lock
            "npm",
            &["audit"],
            "npm is required for bun audit. Install Node.js from https://nodejs.org",
        ),
        "uv" | "pip" | "poetry" => (
            "pip-audit",
            &[] as &[&str],
            "pip-audit is required: pip install pip-audit  or  uv tool install pip-audit",
        ),
        "cargo" => (
            "cargo",
            &["audit"],
            "cargo-audit is required: cargo install cargo-audit",
        ),
        "go" => (
            "govulncheck",
            &["./..."],
            "govulncheck is required: go install golang.org/x/vuln/cmd/govulncheck@latest",
        ),
        _ => (
            "npm",
            &["audit"],
            "unknown package manager; defaulting to npm audit",
        ),
    }
}

/// Detect the package manager from lockfiles present in the workspace root.
/// Fallback when no `package_manager:` section is configured.
fn detect_pm_from_workspace(root: &std::path::Path) -> &'static str {
    if root.join("package-lock.json").exists() || root.join("npm-shrinkwrap.json").exists() {
        return "npm";
    }
    if root.join("yarn.lock").exists() {
        return "yarn";
    }
    if root.join("pnpm-lock.yaml").exists() {
        return "pnpm";
    }
    if root.join("bun.lockb").exists() || root.join("bun.lock").exists() {
        return "bun";
    }
    if root.join("uv.lock").exists() {
        return "uv";
    }
    if root.join("poetry.lock").exists() {
        return "poetry";
    }
    if root.join("requirements.txt").exists() {
        return "pip";
    }
    if root.join("Cargo.lock").exists() {
        return "cargo";
    }
    if root.join("go.sum").exists() {
        return "go";
    }
    "npm"
}
