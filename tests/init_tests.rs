use sbox::init::render_template;

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
    assert!(template.contains("ref: python:3.13-slim"));
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
