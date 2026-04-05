use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::cli::ShimCommand;
use crate::error::SboxError;

/// Package managers and runtimes that sbox knows how to intercept.
/// Install-time tools (npm, pip, ...) catch supply-chain attacks at the source.
/// Runtime tools (node, python3, go, ...) close the post-install artifact gap:
/// code planted in node_modules/.bin during install can't run unsandboxed if `node` is shimmed.
const SHIM_TARGETS: &[&str] = &[
    // package managers / installers
    "npm", "npx", "pnpm", "yarn", "bun", "uv", "pip", "pip3", "poetry", "cargo", "composer",
    // runtimes — prevent post-install artifacts from running on the bare host
    "node", "python3", "python", "go",
];

pub fn execute(command: &ShimCommand) -> Result<ExitCode, SboxError> {
    let shim_dir = resolve_shim_dir(command)?;

    if !command.dry_run {
        fs::create_dir_all(&shim_dir).map_err(|source| SboxError::InitWrite {
            path: shim_dir.clone(),
            source,
        })?;
    }

    let mut created = 0usize;
    let mut skipped = 0usize;

    for &name in SHIM_TARGETS {
        let dest = shim_file_path(&shim_dir, name);

        if dest.exists() && !command.force && !command.dry_run {
            println!(
                "skip   {} (already exists; use --force to overwrite)",
                dest.display()
            );
            skipped += 1;
            continue;
        }

        let real_binary = find_real_binary(name, &shim_dir);
        let script = render_shim(name, real_binary.as_deref());

        if command.dry_run {
            match &real_binary {
                Some(p) => println!("would create {} -> {}", dest.display(), p.display()),
                None => println!("would create {} (real binary not found)", dest.display()),
            }
            created += 1;
            continue;
        }

        fs::write(&dest, &script).map_err(|source| SboxError::InitWrite {
            path: dest.clone(),
            source,
        })?;

        set_executable(&dest).map_err(|source| SboxError::InitWrite {
            path: dest.clone(),
            source,
        })?;

        match &real_binary {
            Some(p) => println!("created {} -> {}", dest.display(), p.display()),
            None => println!(
                "created {} (real binary not found at shim time)",
                dest.display()
            ),
        }
        created += 1;
    }

    if !command.dry_run {
        println!();
        if created > 0 {
            println!(
                "Add {} to your PATH before the real package manager binaries:",
                shim_dir.display()
            );
            println!();
            #[cfg(windows)]
            println!("  set PATH={};%PATH%", shim_dir.display());
            #[cfg(not(windows))]
            println!("  export PATH=\"{}:$PATH\"", shim_dir.display());
            println!();
            #[cfg(not(windows))]
            println!("Then restart your shell or run: source ~/.bashrc");
            #[cfg(windows)]
            println!("Then restart your terminal.");
        }
        if skipped > 0 {
            println!("({skipped} skipped — use --force to overwrite)");
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// Return the full path for a shim file, including platform-specific extension.
fn shim_file_path(dir: &Path, name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        dir.join(format!("{name}.cmd"))
    }
    #[cfg(not(windows))]
    {
        dir.join(name)
    }
}

fn resolve_shim_dir(command: &ShimCommand) -> Result<PathBuf, SboxError> {
    if let Some(dir) = &command.dir {
        let abs = if dir.is_absolute() {
            dir.clone()
        } else {
            std::env::current_dir()
                .map_err(|source| SboxError::CurrentDirectory { source })?
                .join(dir)
        };
        return Ok(abs);
    }

    // Default: ~/.local/bin  (Unix) or %USERPROFILE%\.local\bin  (Windows)
    if let Some(home) = crate::platform::home_dir() {
        return Ok(home.join(".local").join("bin"));
    }

    // Last resort: use current directory
    std::env::current_dir().map_err(|source| SboxError::CurrentDirectory { source })
}

/// Search PATH for `name`, skipping `exclude_dir` to avoid resolving the shim itself.
fn find_real_binary(name: &str, exclude_dir: &Path) -> Option<PathBuf> {
    let path_os = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_os) {
        if dir == exclude_dir {
            continue;
        }
        // On Windows also search for name.exe / name.cmd / name.bat
        #[cfg(windows)]
        {
            for ext in &["", ".exe", ".cmd", ".bat"] {
                let candidate = dir.join(format!("{name}{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        #[cfg(not(windows))]
        {
            let candidate = dir.join(name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && (m.permissions().mode() & 0o111) != 0)
        .unwrap_or(false)
}

/// Make a file executable. On Unix sets the rwxr-xr-x permission bits.
/// On Windows files are executable by virtue of being writable; this is a no-op.
fn set_executable(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    #[cfg(windows)]
    {
        let _ = path; // Windows: executable by extension (.cmd), nothing to set
    }
    Ok(())
}

/// Render a shim script for the given package manager name.
///
/// - Unix: POSIX `/bin/sh` script that walks up the directory tree looking for `sbox.yaml`.
/// - Windows: `.cmd` batch script with equivalent walk-up logic.
fn render_shim(name: &str, real_binary: Option<&Path>) -> String {
    #[cfg(not(windows))]
    return render_shim_posix(name, real_binary);

    #[cfg(windows)]
    return render_shim_cmd(name, real_binary);
}

/// POSIX shell shim (Unix / macOS / Linux).
#[cfg(not(windows))]
fn render_shim_posix(name: &str, real_binary: Option<&Path>) -> String {
    let fallback = match real_binary {
        Some(path) => format!(
            "printf 'sbox: no sbox.yaml found — running {name} unsandboxed\\n' >&2\nexec {path} \"$@\"",
            path = path.display()
        ),
        None => format!(
            "printf 'sbox shim: {name}: real binary not found; reinstall or run `sbox shim` again\\n' >&2\nexit 127"
        ),
    };

    // Note: the ${_sbox_d%/*} shell parameter expansion is written literally here.
    // It strips the last path component, walking up the directory tree.
    format!(
        "#!/bin/sh\n\
         # sbox shim: {name}\n\
         # Generated by `sbox shim`. Re-run `sbox shim --dir DIR` to regenerate.\n\
         _sbox_d=\"$PWD\"\n\
         while true; do\n\
         \x20 if [ -f \"$_sbox_d/sbox.yaml\" ]; then\n\
         \x20   exec sbox run -- {name} \"$@\"\n\
         \x20 fi\n\
         \x20 [ \"$_sbox_d\" = \"/\" ] && break\n\
         \x20 _sbox_d=\"${{_sbox_d%/*}}\"\n\
         \x20 [ -z \"$_sbox_d\" ] && _sbox_d=\"/\"\n\
         done\n\
         {fallback}\n"
    )
}

/// Windows CMD (.cmd) shim.
#[cfg(windows)]
fn render_shim_cmd(name: &str, real_binary: Option<&Path>) -> String {
    let fallback = match real_binary {
        Some(path) => format!(
            "echo sbox: no sbox.yaml found -- running {name} unsandboxed 1>&2\r\n\"{path}\" %*\r\nexit /b %ERRORLEVEL%",
            path = path.display()
        ),
        None => format!(
            "echo sbox shim: {name}: real binary not found; reinstall or run `sbox shim` again 1>&2\r\nexit /b 127"
        ),
    };

    format!(
        "@echo off\r\n\
         :: sbox shim: {name}\r\n\
         :: Generated by `sbox shim`. Re-run `sbox shim --dir DIR` to regenerate.\r\n\
         setlocal enabledelayedexpansion\r\n\
         set \"_sbox_d=%CD%\"\r\n\
         :_sbox_walk_{name}\r\n\
         if exist \"%_sbox_d%\\sbox.yaml\" (\r\n\
         \x20   sbox run -- {name} %*\r\n\
         \x20   exit /b %ERRORLEVEL%\r\n\
         )\r\n\
         for %%P in (\"%_sbox_d%\\..\") do set \"_sbox_parent=%%~fP\"\r\n\
         if \"!_sbox_parent!\"==\"!_sbox_d!\" goto _sbox_fallback_{name}\r\n\
         set \"_sbox_d=!_sbox_parent!\"\r\n\
         goto _sbox_walk_{name}\r\n\
         :_sbox_fallback_{name}\r\n\
         {fallback}\r\n"
    )
}


#[cfg(test)]
mod tests {
    use super::render_shim;

    #[test]
    fn shim_contains_sbox_run_delegation() {
        let script = render_shim("npm", Some(std::path::Path::new("/usr/bin/npm")));
        assert!(script.contains("sbox run -- npm"));
        assert!(script.contains("sbox.yaml"));
    }

    #[test]
    fn shim_fallback_when_real_binary_missing() {
        let script = render_shim("npm", None);
        assert!(script.contains("real binary not found"));
        #[cfg(not(windows))]
        assert!(script.contains("exit 127"));
        #[cfg(windows)]
        assert!(script.contains("exit /b 127"));
    }

    #[test]
    fn shim_walks_to_root() {
        let script = render_shim("uv", Some(std::path::Path::new("/usr/local/bin/uv")));
        #[cfg(not(windows))]
        {
            assert!(script.contains("_sbox_d%/*"));
            assert!(script.contains("[ \"$_sbox_d\" = \"/\" ] && break"));
        }
        #[cfg(windows)]
        {
            assert!(script.contains("_sbox_parent"));
            assert!(script.contains("goto _sbox_walk_uv"));
        }
    }

    /// Verify the generated .cmd script contains all the structural elements needed
    /// to walk up the directory tree and fall back to the real binary.
    #[cfg(windows)]
    #[test]
    fn cmd_shim_structure() {
        let script =
            render_shim("npm", Some(std::path::Path::new(r"C:\Program Files\nodejs\npm.cmd")));

        // Must be a batch file
        assert!(script.contains("@echo off"), "must suppress echo");

        // Must check for sbox.yaml and delegate
        assert!(script.contains("sbox.yaml"), "must check for sbox.yaml");
        assert!(script.contains("sbox run -- npm %*"), "must delegate to sbox run");

        // Walk-up logic: parent dir extraction and loop label
        assert!(
            script.contains("goto _sbox_walk_npm"),
            "must have a labelled walk loop"
        );
        assert!(
            script.contains("_sbox_parent"),
            "must compute parent directory"
        );

        // Fallback to the real binary when no sbox.yaml is found
        assert!(
            script.contains(r"C:\Program Files\nodejs\npm.cmd"),
            "must reference real binary path"
        );
    }

    #[cfg(windows)]
    #[test]
    fn cmd_shim_fallback_when_no_real_binary() {
        let script = render_shim("uv", None);
        assert!(script.contains("real binary not found"));
        assert!(script.contains("exit /b 127"));
    }
}
