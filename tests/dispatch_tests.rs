use sbox::dispatch::{command_string, matches};

#[test]
fn command_string_joins_with_single_space() {
    assert_eq!(
        command_string(&["cargo".into(), "build".into()]),
        "cargo build"
    );
    assert_eq!(
        command_string(&["npm".into(), "install".into(), "--save".into()]),
        "npm install --save"
    );
}

#[test]
fn command_string_handles_empty_arguments() {
    assert_eq!(command_string(&[]), "");
    assert_eq!(command_string(&["single".into()]), "single");
}

#[test]
fn exact_match_requires_full_string_equality() {
    assert!(matches("npm install", "npm install"));
    assert!(!matches("npm install", "npm install lodash"));
    assert!(!matches("npm install", "npm"));
}

#[test]
fn wildcard_prefix_matches_substring() {
    assert!(matches("*install", "npm install"));
    assert!(matches("*install", "cargo install"));
    assert!(matches("*install", "npm uninstall"));
}

#[test]
fn wildcard_suffix_requires_match_at_end() {
    assert!(matches("npm *", "npm install lodash"));
    assert!(matches("npm *", "npm install"));
}

#[test]
fn wildcard_both_ends_matches_substring() {
    assert!(matches("*install*", "npm install"));
    assert!(matches("*install*", "cargo install"));
    assert!(matches("*install*", "pip install --upgrade pip install"));
    assert!(matches("*install*", "npm uninstall"));
}

#[test]
fn wildcard_middle_matches_substring() {
    assert!(matches("npm * install", "npm ci install"));
    assert!(matches("npm * install", "npm install install"));
    assert!(!matches("npm * install", "npm ci"));
}

#[test]
fn multiple_wildcards_work_together() {
    assert!(matches("cargo * *", "cargo install --help"));
    assert!(matches("npm * --save", "npm install lodash --save"));
}

#[test]
fn pure_wildcard_matches_anything() {
    assert!(matches("*", "anything"));
    assert!(matches("*", ""));
    assert!(matches("*", "cargo npm pip python"));
}

#[test]
fn special_regex_characters_are_literal() {
    assert!(matches("npm install.", "npm install."));
    assert!(matches("pip install[", "pip install["));
    assert!(matches("curl http://", "curl http://"));
}

#[test]
fn cargo_build_patterns() {
    assert!(matches("cargo build", "cargo build"));
    assert!(matches("cargo build*", "cargo build"));
    assert!(matches("cargo build*", "cargo build --release"));
    assert!(matches(
        "cargo build*",
        "cargo build --release --target x86_64"
    ));
    assert!(!matches("cargo build*", "cargo test"));
}

#[test]
fn python_patterns() {
    assert!(matches("python *", "python -m pytest"));
    assert!(matches("python *", "python script.py"));
    assert!(matches("pip *", "pip install requests"));
    assert!(matches("uv *", "uv sync"));
}

#[test]
fn node_patterns() {
    assert!(matches("npm *", "npm install"));
    assert!(matches("npm *", "npm ci"));
    assert!(matches("npm *", "npm run build"));
    assert!(matches("pnpm *", "pnpm install"));
    assert!(matches("yarn *", "yarn install"));
}

#[test]
fn case_sensitive_matching() {
    assert!(matches("Cargo Build", "Cargo Build"));
    assert!(!matches("Cargo Build", "cargo build"));
}
