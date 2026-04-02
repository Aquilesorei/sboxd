use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::Duration;
use tempfile::TempDir;

fn podman_tests_enabled() -> bool {
    std::env::var("SBOX_RUN_PODMAN_TESTS").ok().as_deref() == Some("1")
}

fn require_podman_tests() -> bool {
    if podman_tests_enabled() {
        true
    } else {
        eprintln!("skipping Podman integration test; set SBOX_RUN_PODMAN_TESTS=1 to enable");
        false
    }
}

fn signed_verification_test_image() -> Option<String> {
    std::env::var("SBOX_SIGNED_TEST_IMAGE").ok().filter(|value| !value.is_empty())
}

fn require_signed_verification_test() -> Option<String> {
    if !podman_tests_enabled() {
        eprintln!("skipping signed verification test; set SBOX_RUN_PODMAN_TESTS=1 to enable");
        return None;
    }

    let image = match signed_verification_test_image() {
        Some(image) => image,
        None => {
            eprintln!(
                "skipping signed verification test; set SBOX_SIGNED_TEST_IMAGE to a known-good signed image reference"
            );
            return None;
        }
    };

    if std::env::var_os("SBOX_SIGNATURE_POLICY").is_none() {
        eprintln!(
            "skipping signed verification test; set SBOX_SIGNATURE_POLICY to a verification-capable containers policy"
        );
        return None;
    }

    Some(image)
}

fn podman_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
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

fn run_podman(args: &[&str]) -> std::process::Output {
    Command::new("podman")
        .args(args)
        .output()
        .expect("podman command should start")
}

fn reserve_host_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("should reserve ephemeral port");
    let port = listener
        .local_addr()
        .expect("listener should expose local addr")
        .port();
    drop(listener);
    port
}

fn write_temp_config(dir: &TempDir, content: &str) -> PathBuf {
    let path = dir.path().join("sbox.yaml");
    fs::write(&path, content).expect("temp config should be written");
    path
}

fn wait_for_http_200(host_port: u16) -> bool {
    for _ in 0..20 {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", host_port)) {
            let _ = stream.write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n");
            let mut response = String::new();
            if stream.read_to_string(&mut response).is_ok()
                && response.starts_with("HTTP/1.0 200")
            {
                return true;
            }
        }
        sleep(Duration::from_millis(250));
    }

    false
}

fn session_name_from_plan(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    text.lines()
        .find_map(|line| line.trim().strip_prefix("reusable_session: "))
        .filter(|value| *value != "<none>")
        .map(str::to_string)
        .expect("plan output should include a reusable session name")
}

#[test]
fn podman_run_preserves_workspace_cwd_and_mounts() {
    if !require_podman_tests() {
        return;
    }
    let _guard = podman_test_lock()
        .lock()
        .expect("podman integration lock should not be poisoned");

    let project_dir = repo_root().join("examples/python-smoke");

    let status = run_sbox(
        &project_dir,
        &[
            "run",
            "--",
            "python",
            "-c",
            "import json, os; print(json.dumps({'cwd': os.getcwd(), 'app_mode': os.environ.get('APP_MODE')}))",
        ],
    );
    assert!(
        status.status.success(),
        "sbox run should succeed: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    let json = String::from_utf8(status.stdout).expect("stdout should be valid UTF-8");
    let parsed: serde_json::Value =
        serde_json::from_str(&json).expect("sandbox output should be valid JSON");

    assert_eq!(
        parsed.get("cwd").and_then(|value| value.as_str()),
        Some("/workspace/examples/python-smoke")
    );
    assert_eq!(
        parsed.get("app_mode").and_then(|value| value.as_str()),
        Some("sandbox-test")
    );
}

#[test]
fn podman_default_profile_disables_network() {
    if !require_podman_tests() {
        return;
    }
    let _guard = podman_test_lock()
        .lock()
        .expect("podman integration lock should not be poisoned");

    let project_dir = repo_root().join("examples/python-smoke");
    let output = run_sbox(
        &project_dir,
        &[
            "run",
            "--",
            "python",
            "-c",
            "import socket; socket.create_connection(('1.1.1.1', 53), 1)",
        ],
    );

    assert!(
        !output.status.success(),
        "network-disabled profile should fail outbound connection"
    );
}

#[test]
fn podman_reusable_session_can_be_reused_and_cleaned() {
    if !require_podman_tests() {
        return;
    }
    let _guard = podman_test_lock()
        .lock()
        .expect("podman integration lock should not be poisoned");

    let project_dir = repo_root().join("examples/python-smoke");
    let config_path = project_dir.join("reuse-sbox.yaml");

    let plan = run_sbox(&project_dir, &["--config", "reuse-sbox.yaml", "plan", "--", "python", "--version"]);
    assert!(
        plan.status.success(),
        "plan should succeed: {}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let session_name = session_name_from_plan(&plan.stdout);

    let _ = run_podman(&["rm", "-f", &session_name]);

    let first_run =
        run_sbox(&project_dir, &["--config", "reuse-sbox.yaml", "run", "--", "python", "--version"]);
    assert!(
        first_run.status.success(),
        "first reusable run should succeed: {}",
        String::from_utf8_lossy(&first_run.stderr)
    );

    let inspect_running = run_podman(&["inspect", "-f", "{{.State.Running}}", &session_name]);
    assert!(
        inspect_running.status.success(),
        "reusable container should exist after first run: {}",
        String::from_utf8_lossy(&inspect_running.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&inspect_running.stdout).trim(), "true");

    let second_run =
        run_sbox(&project_dir, &["--config", "reuse-sbox.yaml", "run", "--", "python", "--version"]);
    assert!(
        second_run.status.success(),
        "second reusable run should succeed: {}",
        String::from_utf8_lossy(&second_run.stderr)
    );

    let clean = run_sbox(&project_dir, &["--config", "reuse-sbox.yaml", "clean"]);
    assert!(
        clean.status.success(),
        "clean should succeed: {}",
        String::from_utf8_lossy(&clean.stderr)
    );

    let inspect_missing = run_podman(&["inspect", &session_name]);
    assert!(
        !inspect_missing.status.success(),
        "clean should remove the reusable container for {} using {}",
        session_name,
        config_path.display()
    );
}

#[test]
fn podman_reusable_session_exposes_configured_ports() {
    if !require_podman_tests() {
        return;
    }
    let _guard = podman_test_lock()
        .lock()
        .expect("podman integration lock should not be poisoned");

    let root = repo_root();
    let host_port = reserve_host_port();
    let temp = TempDir::new().expect("temp dir should be created");
    let config = format!(
        "version: 1\n\nruntime:\n  backend: podman\n  rootless: true\n  reuse_container: true\n\nworkspace:\n  root: {root}\n  mount: /workspace\n  writable: true\n\nimage:\n  ref: python:3.13-slim\n\nprofiles:\n  default:\n    mode: sandbox\n    network: on\n    writable: true\n    ports:\n      - \"{host_port}:8000\"\n    no_new_privileges: true\n",
        root = root.display(),
        host_port = host_port
    );
    let config_path = write_temp_config(&temp, &config);

    let plan = run_sbox(
        &root,
        &[
            "--config",
            config_path.to_str().expect("temp config path should be UTF-8"),
            "plan",
            "--",
            "python",
            "--version",
        ],
    );
    assert!(
        plan.status.success(),
        "plan should succeed: {}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let session_name = session_name_from_plan(&plan.stdout);
    let _ = run_podman(&["rm", "-f", &session_name]);

    let start_server = run_sbox(
        &root,
        &[
            "--config",
            config_path.to_str().expect("temp config path should be UTF-8"),
            "run",
            "--",
            "sh",
            "-lc",
            "python -m http.server 8000 --bind 0.0.0.0 >/tmp/sbox-http.log 2>&1 &",
        ],
    );
    assert!(
        start_server.status.success(),
        "server bootstrap should succeed: {}",
        String::from_utf8_lossy(&start_server.stderr)
    );

    assert!(
        wait_for_http_200(host_port),
        "expected HTTP server to become reachable on 127.0.0.1:{host_port}"
    );

    let clean = run_sbox(
        &root,
        &[
            "--config",
            config_path.to_str().expect("temp config path should be UTF-8"),
            "clean",
        ],
    );
    assert!(
        clean.status.success(),
        "clean should succeed: {}",
        String::from_utf8_lossy(&clean.stderr)
    );
}

#[test]
fn podman_can_run_with_real_signature_verification() {
    let image = match require_signed_verification_test() {
        Some(image) => image,
        None => return,
    };
    let _guard = podman_test_lock()
        .lock()
        .expect("podman integration lock should not be poisoned");

    let root = repo_root();
    let temp = TempDir::new().expect("temp dir should be created");
    let config = format!(
        "version: 1\n\nruntime:\n  backend: podman\n  rootless: true\n  reuse_container: false\n\nworkspace:\n  root: {root}\n  mount: /workspace\n  writable: true\n\nimage:\n  ref: {image}\n  verify_signature: true\n\nprofiles:\n  default:\n    mode: sandbox\n    network: on\n    writable: true\n    no_new_privileges: true\n    ports: []\n",
        root = root.display(),
        image = image
    );
    let config_path = write_temp_config(&temp, &config);
    let config_arg = config_path.to_str().expect("temp config path should be UTF-8");

    let doctor = run_sbox(&root, &["--config", config_arg, "doctor"]);
    assert!(
        doctor.status.success(),
        "doctor should confirm signature verification readiness: {}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    let doctor_stdout =
        String::from_utf8(doctor.stdout).expect("doctor stdout should be valid UTF-8");
    assert!(
        doctor_stdout.contains("PASS signature-verify"),
        "doctor should report PASS signature-verify, got:\n{doctor_stdout}"
    );

    let run = run_sbox(
        &root,
        &[
            "--config",
            config_arg,
            "run",
            "--",
            "sh",
            "-lc",
            "true",
        ],
    );
    assert!(
        run.status.success(),
        "signed image run should succeed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
}
