use crate::error::{Result, SboxError};
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

const SHIM_TARGETS: &[&str] = &[
    "npm", "npx", "pnpm", "yarn", "bun", "uv", "pip", "pip3", "poetry", "cargo", "composer",
    "bundle", "node", "python3", "python", "go", "ruby",
];

pub fn install() -> Result<()> {
    let shim_dir = get_shim_dir()?;
    fs::create_dir_all(&shim_dir)?;

    let current_exe = env::current_exe()?;
    let sbox_bin = current_exe.to_string_lossy();

    println!("Installing sbox shims into {}", shim_dir.display());

    for target in SHIM_TARGETS {
        let shim_path = shim_dir.join(target);

        let script = format!(
            "#!/bin/sh\nexec \"{}\" run -- {} \"$@\"\n",
            sbox_bin, target
        );

        fs::write(&shim_path, script)?;

        let mut perms = fs::metadata(&shim_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&shim_path, perms)?;

        println!("  ✓ Created shim for {}", target);
    }

    println!("\nTo activate shims, add this to your shell profile (~/.bashrc or ~/.zshrc):");
    println!("  export PATH=\"{}:$PATH\"", shim_dir.display());

    Ok(())
}

pub fn verify() -> Result<()> {
    let shim_dir = get_shim_dir()?;
    let path_var = env::var("PATH").unwrap_or_default();

    let shim_dir_str = shim_dir.to_string_lossy();
    if path_var.contains(&*shim_dir_str) {
        println!(
            "✓ sbox shim directory ({}) is active in PATH.",
            shim_dir.display()
        );
    } else {
        println!(
            "✗ sbox shim directory ({}) is NOT in PATH.",
            shim_dir.display()
        );
        println!(
            "Add it by running: export PATH=\"{}:$PATH\"",
            shim_dir.display()
        );
    }

    Ok(())
}

fn get_shim_dir() -> Result<PathBuf> {
    let home = env::var("HOME")
        .map_err(|_| SboxError::Execution("HOME environment variable not set".to_string()))?;
    Ok(PathBuf::from(home).join(".local/share/sbox/shims"))
}
