use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn sbox_bin() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_sbox")
        .map(PathBuf::from)
        .expect("cargo should expose the built sbox binary to integration tests")
}

fn run_sbox(current_dir: &Path, args: &[&str]) -> String {
    let output = Command::new(sbox_bin())
        .current_dir(current_dir)
        .args(args)
        .output()
        .expect("sbox command should start");

    assert!(
        output.status.success(),
        "sbox command should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("stdout should be valid UTF-8")
}

fn normalize_output(output: &str) -> String {
    let root = repo_root();
    let output = output.replace(root.to_string_lossy().as_ref(), "$ROOT");
    // Replace resolved IP addresses in network_allow lines with a stable placeholder,
    // since DNS resolution is non-deterministic across environments.
    let mut result = Vec::new();
    for line in output.lines() {
        if line.trim_start().starts_with("note: `network_allow` is hostname/DNS-based") {
            continue;
        }
        // network_allow lines already use stable `[resolved] host1, host2` or `[patterns] *.x`
        // format without raw IPs — no normalization needed.
        result.push(line.to_string());
    }
    result.join("\n") + if output.ends_with('\n') { "\n" } else { "" }
}

fn assert_matches_fixture(actual: &str, fixture_name: &str) {
    let fixture_path = repo_root().join("tests/golden").join(fixture_name);
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        fs::write(&fixture_path, actual).expect("should write golden fixture");
        return;
    }
    let expected = fs::read_to_string(&fixture_path).expect("fixture should exist");
    assert_eq!(
        actual,
        expected,
        "golden output mismatch for {}",
        fixture_path.display()
    );
}

#[test]
fn uv_sync_plan_matches_golden_output() {
    let actual = run_sbox(&repo_root(), &["plan", "--", "uv", "sync"]);
    assert_matches_fixture(&normalize_output(&actual), "plan_uv_sync.txt");
}

#[test]
fn npm_install_plan_matches_golden_output() {
    let actual = run_sbox(
        &repo_root(),
        &[
            "--config",
            "examples/npm-smoke/sbox.yaml",
            "plan",
            "--",
            "npm",
            "install",
            "--global",
            "/var/tmp/sbox/npm-artifacts/npm-smoke-0.1.0.tgz",
        ],
    );
    assert_matches_fixture(&normalize_output(&actual), "plan_npm_install_global.txt");
}

#[test]
fn npm_package_add_plan_matches_golden_output() {
    let actual = run_sbox(&repo_root(), &["plan", "--", "npm", "install"]);
    assert_matches_fixture(
        &normalize_output(&actual),
        "plan_npm_install_missing_lockfile.txt",
    );
}

#[test]
fn bun_install_plan_matches_golden_output() {
    let example_dir = repo_root().join("examples/bun-smoke");
    let actual = run_sbox(
        &example_dir,
        &[
            "--config",
            "sbox.yaml",
            "plan",
            "--",
            "bun",
            "install",
            "--ignore-scripts",
        ],
    );
    assert_matches_fixture(&normalize_output(&actual), "plan_bun_install.txt");
}

#[test]
fn npm_preset_install_plan_matches_golden_output() {
    let example_dir = repo_root().join("examples/npm-preset");
    let actual = run_sbox(&example_dir, &["plan", "--", "npm", "install"]);
    assert_matches_fixture(&normalize_output(&actual), "plan_npm_preset_install.txt");
}

#[test]
fn poetry_install_plan_matches_golden_output() {
    let example_dir = repo_root().join("examples/poetry-smoke");
    let actual = run_sbox(
        &example_dir,
        &["--config", "sbox.yaml", "plan", "--", "poetry", "install"],
    );
    assert_matches_fixture(&normalize_output(&actual), "plan_poetry_install.txt");
}
