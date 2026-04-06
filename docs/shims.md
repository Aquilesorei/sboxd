# Transparent Interception with Shims

Shims let you use sbox without changing your existing workflow. Once installed, running `npm install` in a project with `sbox.yaml` automatically delegates to `sbox run -- npm install`. In a project without `sbox.yaml`, the real binary runs unchanged.

## Installing shims

```bash
sbox shim                        # install to ~/.local/bin (default)
sbox shim --dir ~/bin            # custom directory
sbox shim --force                # overwrite existing shims
sbox shim --dry-run              # preview what would be created
```

After installing, add the shim directory **before** the real binaries in your PATH:

```bash
# Unix/macOS — add to ~/.bashrc or ~/.zshrc
export PATH="$HOME/.local/bin:$PATH"

# Windows PowerShell — persist for current user
[Environment]::SetEnvironmentVariable(
  "PATH",
  "$env:USERPROFILE\.local\bin;" + [Environment]::GetEnvironmentVariable("PATH", "User"),
  "User"
)
```

## Verifying shims are active

```bash
sbox shim --verify
```

Checks every shim target and reports whether each shim:
- exists in the shim directory
- appears in PATH **before** the real binary

```
shim dir: /home/user/.local/bin

ok       npm          shim is active (PATH position 0 < 3)
ok       npx          shim is active (PATH position 0 < 3)
ok       cargo        shim active (no real binary found elsewhere in PATH)
shadowed pip          real binary at PATH position 1 comes before shim dir
missing  bundle       shim not found at /home/user/.local/bin/bundle

4 ok, 2 problem(s)

Run `sbox shim` to (re)create missing shims, then ensure /home/user/.local/bin is first in PATH.
```

Exit code is 1 if any problems are found. `sbox doctor` also runs this check and surfaces a summary.

## Supported targets

| Shim | Category |
|------|----------|
| `npm`, `npx` | Node.js |
| `pnpm` | Node.js |
| `yarn` | Node.js |
| `bun` | Node.js |
| `uv`, `pip`, `pip3`, `poetry` | Python |
| `cargo` | Rust |
| `composer` | PHP |
| `bundle` | Ruby |
| `node`, `python3`, `python`, `go`, `ruby` | Runtimes |

Runtime shims (`node`, `python3`, etc.) close the post-install artifact gap: code planted in `node_modules/.bin` during install cannot run on the bare host if `node` is shimmed.

## How shims work

### Unix / macOS

Each shim is a small shell script that walks up the directory tree looking for `sbox.yaml`:

```bash
#!/bin/sh
# sbox shim: npm
_sbox_d="$PWD"
while true; do
  if [ -f "$_sbox_d/sbox.yaml" ]; then
    exec sbox run -- npm "$@"
  fi
  [ "$_sbox_d" = "/" ] && break
  _sbox_d="${_sbox_d%/*}"
  [ -z "$_sbox_d" ] && _sbox_d="/"
done
exec /usr/bin/npm "$@"
```

### Windows

On Windows, shims are `.cmd` batch scripts (e.g. `npm.cmd`). Windows executes `.cmd` files automatically because `.CMD` is in `PATHEXT` — running `npm` finds `npm.cmd` transparently:

```batch
@echo off
setlocal enabledelayedexpansion
set _sbox_d=%CD%
:_sbox_walk_npm
if exist "%_sbox_d%\sbox.yaml" (
  sbox run -- npm %*
  exit /b %ERRORLEVEL%
)
...
:_sbox_fallback_npm
"C:\Program Files\nodejs\npm.cmd" %*
exit /b %ERRORLEVEL%
```

The path to the real binary is hardcoded at shim-generation time to avoid PATH lookup loops. If the real binary moves, re-run `sbox shim` to regenerate.

Exit codes, stdin, stdout, and stderr are passed through unchanged. The shim is transparent to the calling process.

## Dispatch rules with shims

When a shim delegates to sbox, the normal dispatch rules apply:

```yaml
dispatch:
  npm-install:
    match:
      - "npm install*"
    profile: install
  npm-run:
    match:
      - "npm run*"
    profile: default
```

Running `npm install` → `install` profile (network on, registry allowlist, lockfile check).
Running `npm run build` → `default` profile (network off).

## Removing shims

```bash
rm ~/.local/bin/npm ~/.local/bin/npx   # individual shims
sbox shim --dry-run                    # list all shim paths first
```

## Team setup

```bash
# System-wide (requires sudo, Unix)
sudo sbox shim --dir /usr/local/bin

# Or document in the project README:
echo "Run: sbox shim && export PATH=\"\$HOME/.local/bin:\$PATH\""
```

Projects without `sbox.yaml` are unaffected — the shim falls through to the real binary.
