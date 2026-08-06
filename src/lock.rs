use crate::error::{Result, SboxError};
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// Computes the workspace hash used for the lock file name
fn get_workspace_hash(cwd: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(cwd.to_string_lossy().as_bytes());
    hex::encode(hasher.finalize())
}

/// Returns the path to the lock file for the current workspace
fn get_lock_file_path() -> Result<PathBuf> {
    let cwd = env::current_dir()?;
    let workspace_hash = get_workspace_hash(&cwd);
    
    let home = env::var("HOME").map_err(|_| SboxError::Io(io::Error::new(io::ErrorKind::NotFound, "HOME not found")))?;
    let locks_dir = PathBuf::from(home).join(".local").join("share").join("sbox").join("locks");
    
    fs::create_dir_all(&locks_dir)?;
    Ok(locks_dir.join(format!("{}.json", workspace_hash)))
}

/// Hashes a single file's contents
fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];
    
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    
    Ok(hex::encode(hasher.finalize()))
}

/// Recursively hashes a directory's contents, sorting by path
fn hash_directory(dir: &Path) -> Result<String> {
    let mut entries = Vec::new();
    
    fn visit_dirs(dir: &Path, base: &Path, entries: &mut Vec<(String, String)>) -> Result<()> {
        if dir.is_dir() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    visit_dirs(&path, base, entries)?;
                } else if path.is_file() {
                    let rel_path = path.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                    let file_hash = hash_file(&path)?;
                    entries.push((rel_path, file_hash));
                }
            }
        }
        Ok(())
    }
    
    visit_dirs(dir, dir, &mut entries)?;
    
    // Sort to ensure deterministic order
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    
    // Combine all hashes
    let mut master_hasher = Sha256::new();
    for (rel_path, hash) in entries {
        master_hasher.update(rel_path.as_bytes());
        master_hasher.update(b"\0");
        master_hasher.update(hash.as_bytes());
        master_hasher.update(b"\0");
    }
    
    Ok(hex::encode(master_hasher.finalize()))
}

/// Discovers the dependency directory (node_modules, .venv, etc.)
fn find_dependency_dir(cwd: &Path) -> Option<PathBuf> {
    for dir_name in &["node_modules", ".venv", "vendor"] {
        let dir_path = cwd.join(dir_name);
        if dir_path.exists() && dir_path.is_dir() {
            return Some(dir_path);
        }
    }
    None
}

/// Computes the overall hash of the project dependencies
fn compute_project_hash() -> Result<Option<String>> {
    let cwd = env::current_dir()?;
    let dep_dir = match find_dependency_dir(&cwd) {
        Some(dir) => dir,
        None => return Ok(None),
    };
    
    let hash = hash_directory(&dep_dir)?;
    Ok(Some(hash))
}

pub fn lock_project() -> Result<()> {
    let lock_file = get_lock_file_path()?;
    
    println!("[sbox] Computing integrity hash for project dependencies...");
    match compute_project_hash()? {
        Some(hash) => {
            let json = serde_json::json!({
                "version": 1,
                "dependency_hash": hash,
                "timestamp": chrono::Utc::now().to_rfc3339()
            });
            fs::write(&lock_file, serde_json::to_string_pretty(&json).map_err(|e| SboxError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?)?;
            println!("[sbox] Locked project integrity. Hash: {}", &hash[..8]);
        }
        None => {
            println!("[sbox] No dependency directories (node_modules, .venv, vendor) found.");
            if lock_file.exists() {
                fs::remove_file(&lock_file)?;
            }
        }
    }
    
    Ok(())
}

pub fn verify_project() -> Result<()> {
    let lock_file = get_lock_file_path()?;
    
    if !lock_file.exists() {
        let cwd = env::current_dir()?;
        if find_dependency_dir(&cwd).is_some() {
            // There are dependencies but no lockfile
            return Err(SboxError::LockError(
                "Project has dependencies but no sbox lock. Run 'sbox lock' first.".to_string(),
            ));
        }
        return Ok(());
    }
    
    let current_hash = match compute_project_hash()? {
        Some(hash) => hash,
        None => {
            return Err(SboxError::LockError(
                "Project has an sbox lock but dependencies are missing.".to_string(),
            ));
        }
    };
    
    let content = fs::read_to_string(&lock_file)?;
    let json: serde_json::Value = serde_json::from_str(&content).map_err(|e| SboxError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
    
    let locked_hash = json["dependency_hash"].as_str().unwrap_or("");
    
    if current_hash != locked_hash {
        return Err(SboxError::LockError(
            format!(
                "Integrity compromise detected! Dependencies differ from lockfile.\nExpected: {}\nActual: {}\nRun 'sbox lock' to update the lockfile if this is expected.",
                &locked_hash[..8], &current_hash[..8]
            )
        ));
    }
    
    Ok(())
}
