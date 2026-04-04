use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::SboxError;

use super::model::Config;
use super::validate::validate_config;

#[derive(Debug, Clone)]
pub struct LoadOptions {
    pub workspace: Option<PathBuf>,
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub invocation_dir: PathBuf,
    pub workspace_root: PathBuf,
    pub config_path: PathBuf,
    pub config: Config,
}

pub fn load_config(options: &LoadOptions) -> Result<LoadedConfig, SboxError> {
    let invocation_dir =
        std::env::current_dir().map_err(|source| SboxError::CurrentDirectory { source })?;

    let config_path = match options.config.as_deref() {
        Some(path) => absolutize_from(path, &invocation_dir),
        None => {
            let search_root = absolutize_path(options.workspace.as_deref(), &invocation_dir)
                .unwrap_or_else(|| invocation_dir.clone());
            find_default_config_path(&search_root)?
        }
    };

    if !config_path.exists() {
        return Err(SboxError::ConfigNotFound(config_path));
    }

    let raw = fs::read_to_string(&config_path).map_err(|source| SboxError::ConfigRead {
        path: config_path.clone(),
        source,
    })?;

    let mut config = serde_yaml::from_str::<Config>(&raw).map_err(|source| SboxError::ConfigParse {
        path: config_path.clone(),
        source,
    })?;

    super::package_manager::elaborate(&mut config)?;
    validate_config(&config)?;

    let config_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| invocation_dir.clone());

    let workspace_root = match options.workspace.as_deref() {
        Some(path) => absolutize_from(path, &invocation_dir),
        None => {
            let configured_root = config
                .workspace
                .as_ref()
                .and_then(|workspace| workspace.root.as_deref());

            absolutize_path(configured_root, &config_dir).unwrap_or(config_dir)
        }
    };

    Ok(LoadedConfig {
        invocation_dir,
        workspace_root,
        config_path,
        config,
    })
}

fn absolutize_path(path: Option<&Path>, base: &Path) -> Option<PathBuf> {
    path.map(|path| absolutize_from(path, base))
}

fn absolutize_from(path: &Path, base: &Path) -> PathBuf {
    let combined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };

    normalize_path(&combined)
}

fn find_default_config_path(start: &Path) -> Result<PathBuf, SboxError> {
    for directory in start.ancestors() {
        let candidate = directory.join("sbox.yaml");
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(SboxError::ConfigNotFound(start.join("sbox.yaml")))
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}
