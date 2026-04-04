# Getting Started

## Installation

**From crates.io:**

```bash
cargo install sboxd
```

**Pre-built binaries** (Linux x86_64 and aarch64):

```bash
curl -fsSL https://github.com/Aquilesorei/sboxd/releases/latest/download/sbox-linux-x86_64 \
  -o ~/.local/bin/sbox
chmod +x ~/.local/bin/sbox
```

**From source:**

```bash
git clone https://github.com/Aquilesorei/sboxd
cd sboxd
cargo install --path .
```

## Requirements

- Linux (rootless Podman or Docker)
- Podman 4+ or Docker 24+ installed and working

Check everything is ready:

```bash
sbox doctor
```

## Create a config

The fastest way — interactive wizard with arrow-key menus:

```bash
cd myproject
sbox init --interactive
```

Or pick a preset directly:

```bash
sbox init --preset node     # Node.js
sbox init --preset python   # Python
sbox init --preset rust     # Rust
sbox init --preset go       # Go
sbox init --preset generic  # Blank template
```

## Preview the resolved policy

Before running anything, inspect what sbox will actually do:

```bash
sbox plan -- npm install
```

This shows the full resolved `ExecutionPlan`: which image, which mounts, network mode, environment filtering, and which profile was selected — without executing anything.

## Run a command

```bash
sbox run -- npm install
sbox run -- uv sync
sbox run -- cargo build
```

sbox reads `sbox.yaml` from the current directory (or walks up to find one), resolves the policy, and runs the command in a container.

## Run against a specific profile

```bash
sbox exec install -- npm install
sbox exec build -- cargo build --release
```

## Open a shell in the sandbox

```bash
sbox shell
```

## Transparent interception with shims

`sbox shim` writes thin wrapper scripts for common package managers. When called from a project with `sbox.yaml`, they delegate to `sbox run` automatically.

```bash
sbox shim                  # installs to ~/.local/bin
sbox shim --dir ~/bin      # custom directory
sbox shim --dry-run        # preview without writing
```

Add to your shell profile:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

After this, `npm install`, `uv sync`, `bun install`, etc. are automatically sandboxed in any project that has `sbox.yaml`.

## Next steps

- [Network security](network.md) — understanding `network: off` vs `network_allow`
- [Security model](security.md) — what sbox protects and what it does not
- [Config reference](config.md) — full `sbox.yaml` documentation
