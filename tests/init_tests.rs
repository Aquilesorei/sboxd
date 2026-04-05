use sbox::config::{LoadOptions, load_config};
use sbox::init::render_template;
use sbox::resolve::{ResolutionTarget, resolve_execution_plan};
use std::fs;

#[test]
fn generic_preset_uses_ubuntu() {
    let template = render_template("generic").expect("generic preset should exist");
    assert!(template.contains("ref: ubuntu:24.04"));
    assert!(template.contains("backend: podman"));
    assert!(template.contains("mode: sandbox"));
}

#[test]
fn python_preset_uses_python_image() {
    let template = render_template("python").expect("python preset should exist");
    assert!(template.contains("ghcr.io/astral-sh/uv:python3.13-bookworm-slim"));
}

#[test]
fn rust_preset_uses_rust_image() {
    let template = render_template("rust").expect("rust preset should exist");
    assert!(template.contains("ref: rust:1-bookworm"));
}

#[test]
fn node_preset_uses_node_image() {
    let template = render_template("node").expect("node preset should exist");
    assert!(template.contains("ref: node:22-bookworm-slim"));
}

#[test]
fn polyglot_preset_uses_ubuntu() {
    let template = render_template("polyglot").expect("polyglot preset should exist");
    assert!(template.contains("ref: ubuntu:24.04"));
}

#[test]
fn unknown_preset_returns_error() {
    let result = render_template("nonexistent");
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(error.to_string().contains("unknown preset"));
}

#[test]
fn template_contains_required_sections() {
    let template = render_template("generic").expect("generic preset should exist");

    assert!(template.contains("version: 1"));
    assert!(template.contains("runtime:"));
    assert!(template.contains("workspace:"));
    assert!(template.contains("image:"));
    assert!(template.contains("environment:"));
    assert!(template.contains("profiles:"));
    assert!(template.contains("dispatch:"));
}

#[test]
fn template_has_security_defaults() {
    let template = render_template("generic").expect("generic preset should exist");

    assert!(template.contains("rootless: true"));
    assert!(template.contains("network: off"));
    assert!(template.contains("no_new_privileges: true"));
}

#[test]
fn template_has_default_and_host_profiles() {
    let template = render_template("generic").expect("generic preset should exist");

    assert!(template.contains("default:"));
    assert!(template.contains("mode: sandbox"));
    assert!(template.contains("host:"));
    assert!(template.contains("mode: host"));
}

#[test]
fn template_passes_through_term() {
    let template = render_template("generic").expect("generic preset should exist");

    assert!(template.contains("- TERM"));
}

/// End-to-end: generate a preset config, write it to a temp dir, load it, and resolve a plan.
/// Catches regressions where the generated YAML is syntactically valid but fails config
/// validation or plan resolution.
#[test]
fn preset_python_generates_yaml_that_loads_and_resolves() {
    let dir = tempfile::tempdir().expect("temp dir");
    let config_path = dir.path().join("sbox.yaml");

    let yaml = render_template("python").expect("python preset should render");
    fs::write(&config_path, &yaml).expect("write config");

    let loaded = load_config(&LoadOptions {
        workspace: Some(dir.path().to_path_buf()),
        config: Some(config_path),
    })
    .expect("python preset config should load without errors");

    let cli = sbox::cli::Cli {
        config: None,
        workspace: None,
        backend: None,
        image: None,
        profile: None,
        mode: None,
        verbose: 0,
        quiet: false,
        strict_security: false,
        command: sbox::cli::Commands::Doctor(sbox::cli::DoctorCommand::default()),
    };

    // install-style command should resolve via the package_manager preset
    let plan = resolve_execution_plan(
        &cli,
        &loaded,
        ResolutionTarget::Run,
        &["uv".into(), "sync".into()],
    )
    .expect("uv sync should resolve without errors");

    assert_eq!(
        plan.profile_name, "pm-uv-install",
        "should dispatch to preset install profile"
    );
    assert!(
        plan.policy.network == "on",
        "install profile should allow network"
    );
}

#[test]
fn preset_node_generates_yaml_that_loads_and_resolves() {
    let dir = tempfile::tempdir().expect("temp dir");
    let config_path = dir.path().join("sbox.yaml");

    let yaml = render_template("node").expect("node preset should render");
    fs::write(&config_path, &yaml).expect("write config");

    let loaded = load_config(&LoadOptions {
        workspace: Some(dir.path().to_path_buf()),
        config: Some(config_path),
    })
    .expect("node preset config should load without errors");

    let cli = sbox::cli::Cli {
        config: None,
        workspace: None,
        backend: None,
        image: None,
        profile: None,
        mode: None,
        verbose: 0,
        quiet: false,
        strict_security: false,
        command: sbox::cli::Commands::Doctor(sbox::cli::DoctorCommand::default()),
    };

    let plan = resolve_execution_plan(
        &cli,
        &loaded,
        ResolutionTarget::Run,
        &["npm".into(), "install".into()],
    )
    .expect("npm install should resolve without errors");

    assert_eq!(plan.profile_name, "pm-npm-install");
    assert!(plan.policy.network == "on");
}
