use crate::error::{Result, SboxError};
use crate::policy::CommandPolicy;
use landlock::{
    Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, ABI,
};
use std::env;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

const SENSITIVE_ENV_PREFIXES: &[&str] = &[
    "AWS_",
    "GITHUB_TOKEN",
    "NPM_TOKEN",
    "SLACK_TOKEN",
    "SECRET_",
    "PRIVATE_KEY",
    "DATABASE_URL",
    "PASSWORD",
];

pub struct NativeSandbox;

impl NativeSandbox {
    pub fn execute(policy: &CommandPolicy) -> Result<ExitStatus> {
        let mut cmd = Command::new(&policy.binary_path);
        cmd.args(&policy.args);

        // 1. Scrub sensitive environment variables
        let current_envs: Vec<(String, String)> = env::vars().collect();
        for (key, _) in current_envs {
            if Self::is_sensitive_env(&key) {
                cmd.env_remove(&key);
            }
        }

        // 2. Prepare Landlock Ruleset
        let abi = ABI::V1;
        let mut ruleset = Ruleset::default()
            .handle_access(AccessFs::from_all(abi))
            .map_err(|e| SboxError::Landlock(e.to_string()))?
            .create()
            .map_err(|e| SboxError::Landlock(e.to_string()))?;

        // Add Read-Only system directories
        let mut ro_paths = vec![
            PathBuf::from("/usr"),
            PathBuf::from("/lib"),
            PathBuf::from("/lib64"),
            PathBuf::from("/etc"),
            PathBuf::from("/bin"),
            PathBuf::from("/sbin"),
            PathBuf::from("/dev"),
            PathBuf::from("/proc"),
        ];

        // Add home toolchain directories (read-only)
        if let Ok(home) = env::var("HOME") {
            let home_path = PathBuf::from(home);
            for toolchain_dir in &[".rustup", ".cargo", ".nvm", ".pyenv", ".bun", ".local", ".cache"] {
                let path = home_path.join(toolchain_dir);
                if path.exists() {
                    ro_paths.push(path);
                }
            }
        }

        for path in ro_paths {
            if path.exists() {
                if let Ok(fd) = PathFd::new(&path) {
                    ruleset = ruleset
                        .add_rule(PathBeneath::new(fd, AccessFs::from_read(abi)))
                        .map_err(|e| SboxError::Landlock(e.to_string()))?;
                }
            }
        }

        // Add user PATH directories to Read-Only (e.g. ~/.cargo/bin, ~/.nvm)
        if let Ok(path_var) = env::var("PATH") {
            for dir in path_var.split(':') {
                let p = Path::new(dir);
                if p.exists() {
                    if let Ok(fd) = PathFd::new(p) {
                        ruleset = ruleset
                            .add_rule(PathBeneath::new(fd, AccessFs::from_read(abi)))
                            .map_err(|e| SboxError::Landlock(e.to_string()))?;
                    }
                }
            }
        }

        // Add Read-Write paths (CWD, /tmp, etc.)
        for rw_path in &policy.writable_paths {
            if rw_path.exists() {
                if let Ok(fd) = PathFd::new(rw_path) {
                    ruleset = ruleset
                        .add_rule(PathBeneath::new(fd, AccessFs::from_all(abi)))
                        .map_err(|e| SboxError::Landlock(e.to_string()))?;
                }
            }
        }

        // 3. Network Namespace Policy
        let network_enabled = policy.network_enabled;
        let mut ruleset_opt = Some(ruleset);

        // 4. Configure pre_exec hook for Landlock restriction & Network unshare
        unsafe {
            cmd.pre_exec(move || {
                // Network isolation if network is OFF
                if !network_enabled {
                    let ret = libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET);
                    if ret != 0 {
                        // Fallback if unshare fails
                        let _ = libc::unshare(libc::CLONE_NEWNET);
                    }
                }

                // Restrict process with Landlock
                if let Some(r) = ruleset_opt.take() {
                    match r.restrict_self() {
                        Ok(_) => Ok(()),
                        Err(e) => Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            e.to_string(),
                        )),
                    }
                } else {
                    Ok(())
                }
            });
        }

        // 5. Spawn and wait for process
        let status = cmd.status()?;
        Ok(status)
    }

    fn is_sensitive_env(key: &str) -> bool {
        let upper = key.to_uppercase();
        for prefix in SENSITIVE_ENV_PREFIXES {
            if upper.starts_with(prefix)
                || upper.contains("SECRET")
                || upper.contains("TOKEN")
                || upper.contains("KEY")
            {
                return true;
            }
        }
        false
    }
}
