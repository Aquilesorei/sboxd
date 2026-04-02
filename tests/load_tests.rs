use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

use sbox::config::load::{LoadOptions, load_config};

fn write_config(dir: &TempDir, content: &str) -> PathBuf {
    let config_path = dir.path().join("sbox.yaml");
    fs::write(&config_path, content).expect("failed to write test config");
    config_path
}

const MINIMAL_CONFIG_YAML: &str = r#"version: 1

runtime:
  backend: podman
  rootless: true

workspace:
  root: .
  mount: /workspace

image:
  ref: python:3.13-slim

profiles:
  default:
    mode: sandbox
    network: off
    writable: true
"#;

fn setup_load_options(config_path: &PathBuf) -> LoadOptions {
    LoadOptions {
        workspace: None,
        config: Some(config_path.clone()),
    }
}

fn cwd_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn with_current_dir<T>(path: &std::path::Path, f: impl FnOnce() -> T) -> T {
    let _guard = cwd_lock().lock().expect("cwd lock should not be poisoned");
    let original = std::env::current_dir().expect("current dir should be readable");
    std::env::set_current_dir(path).expect("failed to change dir");
    let result = f();
    std::env::set_current_dir(original).expect("failed to restore dir");
    result
}

#[test]
fn loads_valid_config_from_explicit_path() {
    let temp = TempDir::new().expect("failed to create temp dir");
    let config_path = write_config(&temp, MINIMAL_CONFIG_YAML);

    let options = setup_load_options(&config_path);
    let result = load_config(&options).expect("load should succeed");

    assert_eq!(result.config.version, 1);
    assert_eq!(result.config_path, config_path);
}

#[test]
fn finds_config_by_searching_ancestors() {
    let temp = TempDir::new().expect("failed to create temp dir");
    let config_path = write_config(&temp, MINIMAL_CONFIG_YAML);

    let subdir = temp.path().join("subdir").join("deeper");
    fs::create_dir_all(&subdir).expect("failed to create subdirs");

    let result = with_current_dir(&subdir, || {
        let options = LoadOptions {
            workspace: None,
            config: None,
        };
        load_config(&options).expect("load should succeed")
    });

    assert_eq!(result.config_path, config_path);
}

#[test]
fn fails_when_config_does_not_exist() {
    let temp = TempDir::new().expect("failed to create temp dir");
    let config_path = temp.path().join("nonexistent.yaml");

    let options = LoadOptions {
        workspace: None,
        config: Some(config_path.clone()),
    };

    let error = load_config(&options).expect_err("load should fail");
    assert!(error.to_string().contains("not found"));
}

#[test]
fn uses_workspace_override_for_config_search() {
    let temp = TempDir::new().expect("failed to create temp dir");
    let config_path = write_config(&temp, MINIMAL_CONFIG_YAML);

    let other_dir = TempDir::new().expect("failed to create other temp dir");
    let result = with_current_dir(other_dir.path(), || {
        let options = LoadOptions {
            workspace: Some(temp.path().to_path_buf()),
            config: None,
        };

        load_config(&options).expect("load should succeed")
    });
    assert_eq!(result.config_path, config_path);
}

#[test]
fn workspace_root_defaults_to_config_directory() {
    let temp = TempDir::new().expect("failed to create temp dir");
    let config_path = write_config(&temp, MINIMAL_CONFIG_YAML);

    let options = setup_load_options(&config_path);
    let result = load_config(&options).expect("load should succeed");

    assert_eq!(result.workspace_root, temp.path());
}

#[test]
fn uses_explicit_workspace_root_from_config() {
    let temp = TempDir::new().expect("failed to create temp dir");
    let config_path = write_config(&temp, MINIMAL_CONFIG_YAML);

    let options = setup_load_options(&config_path);
    let result = load_config(&options).expect("load should succeed");

    assert_eq!(result.workspace_root, temp.path());
}

#[test]
fn preserves_invocation_directory_separate_from_workspace() {
    let temp = TempDir::new().expect("failed to create temp dir");
    let config_path = write_config(&temp, MINIMAL_CONFIG_YAML);

    let subdir = temp.path().join("subdir");
    fs::create_dir_all(&subdir).expect("failed to create subdir");
    let result = with_current_dir(&subdir, || {
        let options = setup_load_options(&config_path);
        load_config(&options).expect("load should succeed")
    });

    assert_eq!(result.invocation_dir, subdir);
    assert_eq!(result.workspace_root, temp.path());
}
