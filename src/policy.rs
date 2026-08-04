use crate::error::{Result, SboxError};
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct CommandPolicy {
    pub binary_path: PathBuf,
    pub program_name: String,
    pub args: Vec<String>,
    pub network_enabled: bool,
    pub writable_paths: Vec<PathBuf>,
}

impl CommandPolicy {
    pub fn resolve(cmd: &str, args: &[String]) -> Result<Self> {
        let binary_path = which::which(cmd)
            .map_err(|_| SboxError::BinaryNotFound(cmd.to_string()))?;

        let program_name = cmd.to_string();
        let network_enabled = Self::detect_network_policy(cmd, args);
        
        let mut writable_paths = vec![
            env::current_dir()?,
            PathBuf::from("/tmp"),
            PathBuf::from("/var/tmp"),
        ];

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
            writable_paths,
        })
    }

    fn detect_network_policy(cmd: &str, args: &[String]) -> bool {
        let full_cmd_line = format!("{} {}", cmd, args.join(" "));
        let lower = full_cmd_line.to_lowercase();

        // Install / Sync commands require network access
        if lower.contains("install") 
            || lower.contains("sync") 
            || lower.contains("add") 
            || lower.contains("fetch") 
            || lower.contains("update") 
            || lower.contains("get") 
        {
            return true;
        }

        // By default for builds, tests, scripts, and runs: network is OFF
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_network_policy_install() {
        assert!(CommandPolicy::detect_network_policy("npm", &["install".to_string()]));
        assert!(CommandPolicy::detect_network_policy("uv", &["sync".to_string()]));
        assert!(CommandPolicy::detect_network_policy("cargo", &["add".to_string(), "serde".to_string()]));
    }

    #[test]
    fn test_detect_network_policy_build_and_test() {
        assert!(!CommandPolicy::detect_network_policy("cargo", &["build".to_string()]));
        assert!(!CommandPolicy::detect_network_policy("npm", &["test".to_string()]));
        assert!(!CommandPolicy::detect_network_policy("python", &["main.py".to_string()]));
    }
}
