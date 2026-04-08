use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

fn docker_tests_enabled() -> bool {
    std::env::var("SBOX_RUN_DOCKER_TESTS").ok().as_deref() == Some("1")
}

fn require_docker_tests() -> bool {
    if docker_tests_enabled() {
        true
    } else {
        eprintln!("skipping Docker integration test; set SBOX_RUN_DOCKER_TESTS=1 to enable");
        false
    }
}

fn docker_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn acquire_docker_test_lock() -> std::sync::MutexGuard<'static, ()> {
    docker_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn sbox_bin() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_sbox")
        .map(PathBuf::from)
        .expect("cargo should expose the built sbox binary to integration tests")
}

fn run_sbox(current_dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(sbox_bin())
        .current_dir(current_dir)
        .args(args)
        .output()
        .expect("sbox command should start")
}

fn write_temp_config(dir: &TempDir, content: &str) -> PathBuf {
    let path = dir.path().join("sbox.yaml");
    fs::write(&path, content).expect("temp config should be written");
    path
}

// ── basic sandbox execution ───────────────────────────────────────────────────

#[test]
fn docker_run_preserves_workspace_cwd_and_env() {
    if !require_docker_tests() {
        return;
    }
    let _guard = acquire_docker_test_lock();

    let root = repo_root();
    let temp = TempDir::new().expect("temp dir should be created");
    let config = format!(
        "version: 1\n\nruntime:\n  backend: docker\n  rootless: false\n  reuse_container: false\n\nworkspace:\n  root: {root}\n  mount: /workspace\n  writable: true\n\nimage:\n  ref: python:3.13-slim\n\nenvironment:\n  set:\n    APP_MODE: docker-test\n\nprofiles:\n  default:\n    mode: sandbox\n    network: off\n    writable: true\n    no_new_privileges: true\n    ports: []\n",
        root = root.display()
    );
    let config_path = write_temp_config(&temp, &config);

    let output = run_sbox(
        &root,
        &[
            "--config",
            config_path
                .to_str()
                .expect("temp config path should be UTF-8"),
            "run",
            "--",
            "python",
            "-c",
            "import json, os; print(json.dumps({'cwd': os.getcwd(), 'app_mode': os.environ.get('APP_MODE')}))",
        ],
    );
    assert!(
        output.status.success(),
        "sbox run should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = String::from_utf8(output.stdout).expect("stdout should be valid UTF-8");
    let parsed: serde_json::Value =
        serde_json::from_str(json.trim()).expect("sandbox output should be valid JSON");

    assert_eq!(
        parsed.get("cwd").and_then(|v| v.as_str()),
        Some("/workspace"),
        "cwd inside container should be /workspace"
    );
    assert_eq!(
        parsed.get("app_mode").and_then(|v| v.as_str()),
        Some("docker-test"),
        "APP_MODE env var should be set"
    );
}

// ── network isolation ─────────────────────────────────────────────────────────

#[test]
fn docker_network_off_blocks_outbound_connections() {
    if !require_docker_tests() {
        return;
    }
    let _guard = acquire_docker_test_lock();

    let root = repo_root();
    let temp = TempDir::new().expect("temp dir should be created");
    let config = format!(
        "version: 1\n\nruntime:\n  backend: docker\n  rootless: false\n  reuse_container: false\n\nworkspace:\n  root: {root}\n  mount: /workspace\n  writable: true\n\nimage:\n  ref: python:3.13-slim\n\nprofiles:\n  default:\n    mode: sandbox\n    network: off\n    writable: true\n    no_new_privileges: true\n    ports: []\n",
        root = root.display()
    );
    let config_path = write_temp_config(&temp, &config);

    let output = run_sbox(
        &root,
        &[
            "--config",
            config_path
                .to_str()
                .expect("temp config path should be UTF-8"),
            "run",
            "--",
            "python",
            "-c",
            "import socket; socket.create_connection(('1.1.1.1', 53), 1)",
        ],
    );
    assert!(
        !output.status.success(),
        "network-disabled profile should fail outbound TCP connections"
    );
}

#[test]
fn docker_cloud_metadata_endpoint_blocked_with_network_on() {
    if !require_docker_tests() {
        return;
    }
    let _guard = acquire_docker_test_lock();

    let root = repo_root();
    let temp = TempDir::new().expect("temp dir should be created");
    // network: on but no network_allow — metadata denylist should still fire
    let config = format!(
        "version: 1\n\nruntime:\n  backend: docker\n  rootless: false\n  reuse_container: false\n\nworkspace:\n  root: {root}\n  mount: /workspace\n  writable: true\n\nimage:\n  ref: python:3.13-slim\n\nprofiles:\n  default:\n    mode: sandbox\n    network: on\n    writable: true\n    no_new_privileges: true\n    ports: []\n",
        root = root.display()
    );
    let config_path = write_temp_config(&temp, &config);

    // Try to connect to the metadata endpoint by hostname.
    // Docker sinkholes metadata hostnames with --add-host, but cannot intercept
    // a raw hard-coded IP connect() without stronger firewalling.
    let output = run_sbox(
        &root,
        &[
            "--config",
            config_path
                .to_str()
                .expect("temp config path should be UTF-8"),
            "run",
            "--",
            "python",
            "-c",
            "import socket; socket.create_connection(('metadata.google.internal', 80), 1)",
        ],
    );
    assert!(
        !output.status.success(),
        "cloud metadata endpoint should be unreachable even with network: on"
    );
}

#[test]
fn docker_network_allow_blocks_unlisted_hosts_via_dns_break() {
    if !require_docker_tests() {
        return;
    }
    let _guard = acquire_docker_test_lock();

    let root = repo_root();
    let temp = TempDir::new().expect("temp dir should be created");
    // network_allow lists only registry.npmjs.org — example.com must be unresolvable
    let config = format!(
        "version: 1\n\nruntime:\n  backend: docker\n  rootless: false\n  reuse_container: false\n\nworkspace:\n  root: {root}\n  mount: /workspace\n  writable: true\n\nimage:\n  ref: python:3.13-slim\n\nprofiles:\n  default:\n    mode: sandbox\n    network: on\n    network_allow:\n      - registry.npmjs.org\n    writable: true\n    no_new_privileges: true\n    ports: []\n",
        root = root.display()
    );
    let config_path = write_temp_config(&temp, &config);

    // DNS is broken (--dns 192.0.2.1), so example.com must not resolve
    let output = run_sbox(
        &root,
        &[
            "--config",
            config_path
                .to_str()
                .expect("temp config path should be UTF-8"),
            "run",
            "--",
            "python",
            "-c",
            "import socket; socket.getaddrinfo('example.com', 80)",
        ],
    );
    assert!(
        !output.status.success(),
        "unlisted host should not resolve when network_allow is set (DNS break active)"
    );
}

// ── environment variable denial ───────────────────────────────────────────────

#[test]
fn docker_denied_env_vars_not_visible_in_container() {
    if !require_docker_tests() {
        return;
    }
    let _guard = acquire_docker_test_lock();

    let root = repo_root();
    let temp = TempDir::new().expect("temp dir should be created");
    let config = format!(
        "version: 1\n\nruntime:\n  backend: docker\n  rootless: false\n  reuse_container: false\n\nworkspace:\n  root: {root}\n  mount: /workspace\n  writable: true\n\nimage:\n  ref: python:3.13-slim\n\nenvironment:\n  deny:\n    - NPM_TOKEN\n    - NODE_AUTH_TOKEN\n\nprofiles:\n  default:\n    mode: sandbox\n    network: off\n    writable: true\n    no_new_privileges: true\n    ports: []\n",
        root = root.display()
    );
    let config_path = write_temp_config(&temp, &config);

    let output = run_sbox(
        &root,
        &[
            "--config",
            config_path
                .to_str()
                .expect("temp config path should be UTF-8"),
            "run",
            "--",
            "python",
            "-c",
            "import os; print(os.environ.get('NPM_TOKEN', 'not-set'))",
        ],
    );
    assert!(
        output.status.success(),
        "sbox run should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout should be valid UTF-8");
    assert_eq!(
        stdout.trim(),
        "not-set",
        "denied NPM_TOKEN should not be visible inside container"
    );
}

// ── workspace isolation ───────────────────────────────────────────────────────

#[test]
fn docker_workspace_root_is_read_only_without_writable() {
    if !require_docker_tests() {
        return;
    }
    let _guard = docker_test_lock()
        .lock()
        .expect("docker integration lock should not be poisoned");

    let root = repo_root();
    let temp = TempDir::new().expect("temp dir should be created");
    let config = format!(
        "version: 1\n\nruntime:\n  backend: docker\n  rootless: false\n  reuse_container: false\n\nworkspace:\n  root: {root}\n  mount: /workspace\n  writable: false\n\nimage:\n  ref: python:3.13-slim\n\nprofiles:\n  default:\n    mode: sandbox\n    network: off\n    writable: false\n    no_new_privileges: true\n    ports: []\n",
        root = root.display()
    );
    let config_path = write_temp_config(&temp, &config);

    let output = run_sbox(
        &root,
        &[
            "--config",
            config_path
                .to_str()
                .expect("temp config path should be UTF-8"),
            "run",
            "--",
            "python",
            "-c",
            "open('/workspace/sbox_test_write_probe.txt', 'w').write('fail')",
        ],
    );
    assert!(
        !output.status.success(),
        "write to read-only workspace should fail inside Docker container"
    );
}

// ── npm preset via package_manager ───────────────────────────────────────────

#[test]
fn docker_npm_preset_plan_shows_preset_profile_and_token_denials() {
    if !require_docker_tests() {
        return;
    }
    let _guard = docker_test_lock()
        .lock()
        .expect("docker integration lock should not be poisoned");

    let root = repo_root();
    let temp = TempDir::new().expect("temp dir should be created");
    let config = format!(
        "version: 1\n\nruntime:\n  backend: docker\n  rootless: false\n\nworkspace:\n  root: {root}\n  mount: /workspace\n  writable: false\n\nimage:\n  ref: node:22-bookworm-slim\n\npackage_manager:\n  name: npm\n",
        root = root.display()
    );
    let config_path = write_temp_config(&temp, &config);

    let output = run_sbox(
        &root,
        &[
            "--config",
            config_path
                .to_str()
                .expect("temp config path should be UTF-8"),
            "plan",
            "--",
            "npm",
            "install",
        ],
    );
    assert!(
        output.status.success(),
        "sbox plan should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let plan_text = String::from_utf8(output.stdout).expect("stdout should be valid UTF-8");

    assert!(
        plan_text.contains("pm-npm-install"),
        "plan should show preset install profile name"
    );
    assert!(
        plan_text.contains("NPM_TOKEN"),
        "plan should show NPM_TOKEN in denied list"
    );
    assert!(
        plan_text.contains("NODE_AUTH_TOKEN"),
        "plan should show NODE_AUTH_TOKEN in denied list"
    );
    assert!(
        plan_text.contains("registry.npmjs.org"),
        "plan should show npm registry in network_allow"
    );
}
