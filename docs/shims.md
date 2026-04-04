# Transparent Interception with Shims

Shims let you use sbox without changing your existing workflow. Once installed, running `npm install` in a project with `sbox.yaml` automatically delegates to `sbox run -- npm install`. In a project without `sbox.yaml`, the real npm runs unchanged.

## Installing shims

```bash
sbox shim                        # install to ~/.local/bin (default)
sbox shim --dir ~/bin            # custom directory
sbox shim --force                # overwrite existing shims
sbox shim --dry-run              # preview what would be created
```

After installing, add the shim directory before the real binaries in your PATH:

```bash
# Add to ~/.bashrc or ~/.zshrc
export PATH="$HOME/.local/bin:$PATH"
```

Verify the shim is active:

```bash
which npm          # should show ~/.local/bin/npm
npm --version      # calls the shim, which passes through to real npm (no sbox.yaml here)
```

## Supported package managers

| Shim | Intercepts |
|------|-----------|
| `npm` | All npm commands |
| `npx` | npx invocations |
| `pnpm` | All pnpm commands |
| `yarn` | All yarn commands |
| `bun` | All bun commands |
| `uv` | All uv commands |
| `pip` | All pip commands |
| `pip3` | All pip3 commands |
| `poetry` | All poetry commands |
| `cargo` | All cargo commands |
| `composer` | All composer commands |

## How shims work

Each shim is a small shell script:

```bash
#!/bin/sh
# sbox shim — transparent interception for npm
SBOX_REAL_BIN=/usr/bin/npm
if [ -f "$(pwd)/sbox.yaml" ] || sbox_find_config 2>/dev/null; then
    exec sbox run -- npm "$@"
else
    exec "$SBOX_REAL_BIN" "$@"
fi
```

The shim walks up from the current directory looking for `sbox.yaml` (the same search sbox itself does). If found, it delegates to sbox. If not found, the real binary is called with all original arguments.

Exit codes, stdin, stdout, and stderr are passed through unchanged. The shim is transparent to the calling process.

## Dispatch rules with shims

When a shim delegates to sbox, sbox applies the normal dispatch rules. For example, with:

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

Running `npm install` uses the `install` profile (network on, registry allowlist, lockfile check).
Running `npm run build` uses the `default` profile (network off).

## Removing shims

```bash
rm ~/.local/bin/npm ~/.local/bin/npx  # remove individual shims
# or remove all at once:
sbox shim --dry-run                   # list shim paths
```

## Team setup

For a team where everyone should have sbox active:

```bash
# System-wide (requires sudo)
sudo sbox shim --dir /usr/local/bin

# Or document in the project README:
echo "Run: sbox shim && export PATH=\"\$HOME/.local/bin:\$PATH\""
```

Projects without `sbox.yaml` are unaffected.
