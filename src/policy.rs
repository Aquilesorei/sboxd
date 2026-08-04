use crate::error::{Result, SboxError};
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct CommandPolicy {
    pub binary_path: PathBuf,
    pub program_name: String,
    pub args: Vec<String>,
    pub network_enabled: bool,
    pub allow_env: bool,
    pub allow_net_out: bool,
    pub writable_paths: Vec<PathBuf>,
}

impl CommandPolicy {
    pub fn resolve(cmd: &str, args: &[String], offline: bool, allow_env: bool, allow_net_out: bool) -> Result<Self> {
        let binary_path =
            which::which(cmd).map_err(|_| SboxError::BinaryNotFound(cmd.to_string()))?;

        let program_name = cmd.to_string();
        // Network is ON by default so dev tools never break; offline flag cuts it off
        let network_enabled = !offline;

        let mut writable_paths = vec![
            env::current_dir()?,
            PathBuf::from("/tmp"),
            PathBuf::from("/var/tmp"),
        ];

        if let Ok(home) = env::var("HOME") {
            let cache_dir = PathBuf::from(home).join(".cache");
            if cache_dir.exists() {
                writable_paths.push(cache_dir);
            }
        }

        // Ensure target build directories if present are writable
        let cwd = env::current_dir()?;
        for dir in &["node_modules", "target", ".venv", "vendor", "dist", "build"] {
            let path = cwd.join(dir);
            if path.exists() {
                writable_paths.push(path);
            }
        }

        Ok(Self {
            binary_path,
            program_name,
            args: args.to_vec(),
            network_enabled,
            allow_env,
            allow_net_out,
            writable_paths,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_resolve_network_by_default() {
        let policy = CommandPolicy::resolve("echo", &["test".to_string()], false, false, false).unwrap();
        assert!(policy.network_enabled);
    }

    #[test]
    fn test_policy_resolve_offline_flag() {
        let policy = CommandPolicy::resolve("echo", &["test".to_string()], true, false, false).unwrap();
        assert!(!policy.network_enabled);
    }
}
