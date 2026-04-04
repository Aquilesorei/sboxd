# Getting Started

## What you need

- Linux (Fedora, Ubuntu, Debian, Arch, etc.)
- Rootless Podman **or** Docker installed and working
- That's it

If you're not sure whether rootless Podman is set up correctly, run `sbox doctor` after installing — it checks everything and tells you what to fix.

---

## Install sbox

**From crates.io:**

```bash
cargo install sboxd
```

The binary is named `sbox`.

**Pre-built binaries** (no Rust toolchain needed):

```bash
# x86_64
curl -fsSL https://github.com/Aquilesorei/sboxd/releases/latest/download/sbox-linux-x86_64 \
  -o ~/.local/bin/sbox && chmod +x ~/.local/bin/sbox

# aarch64
curl -fsSL https://github.com/Aquilesorei/sboxd/releases/latest/download/sbox-linux-aarch64 \
  -o ~/.local/bin/sbox && chmod +x ~/.local/bin/sbox
```

Make sure `~/.local/bin` is in your PATH:

```bash
export PATH="$HOME/.local/bin:$PATH"   # add to ~/.bashrc or ~/.zshrc
```

**From source:**

```bash
git clone https://github.com/Aquilesorei/sboxd
cd sboxd
cargo install --path .
```

---

## Check everything works

```bash
sbox doctor
```

This tells you whether Podman or Docker is available, whether rootless mode is configured, whether `skopeo` is available for signature verification, and whether shims are installed. Fix anything it flags before continuing.

---

## Add sbox to a project

### Option 1 — preset (fastest)

```bash
sbox init --preset node      # Node.js — npm, node:22-bookworm-slim
sbox init --preset python    # Python  — uv,  python:3.13-slim
sbox init --preset rust      # Rust    — cargo, rust:1-bookworm
sbox init --preset go        # Go      — go, golang:1.23-bookworm
sbox init --preset generic   # Blank   — ubuntu:24.04, manual profiles
```

Language presets generate a `package_manager:` config — one line declares the package manager name and sbox handles the rest. The `generic` preset generates a manual profiles skeleton instead.

### Option 2 — interactive wizard

```bash
cd myproject
sbox init --interactive
```

The wizard asks two to five questions depending on your choice:

**Simple mode** (recommended for most projects):

1. Setup mode → `simple`
2. Package manager → npm / yarn / pnpm / bun / uv / pip / poetry / cargo / go
3. Container image → default shown, press Enter to accept
4. Container backend → auto / podman / docker

Writes a minimal config with `package_manager:`. That's it — sbox infers install profiles, build profiles, network policy, and writable paths from the preset.

**Advanced mode** (for custom policies):

1. Setup mode → `advanced`
2. Container backend
3. Language / ecosystem
4. Container image
5. Default network access
6. Writable paths
7. Whether to add dispatch rules

Writes a config with explicit `profiles:` and `dispatch:` for full manual control.

Press Enter at any prompt to accept the default.

---

## See what will happen before running anything

```bash
sbox plan -- npm install
```

This resolves the full execution policy and prints it — which image, which mounts, which env vars pass through, what network policy — without starting a container. Use this to understand and debug your config.

```
sbox plan
phase: 2
config: /home/user/myproject/sbox.yaml

command: npm install

resolution:
  profile: pm-npm-install
  profile source: package_manager preset `npm` (install) via pattern `npm install*`
  mode: sandbox

runtime:
  backend: podman
  image: ref:node:22-bookworm-slim
  user mapping: keep-id

workspace:
  root: /home/user/myproject
  mount: /workspace
  sandbox cwd: /workspace

policy:
  network: on
  network_allow: [resolved] registry.npmjs.org
  writable: false
  no_new_privileges: true

mounts:
  - bind /home/user/myproject -> /workspace (ro, workspace)
  - bind /home/user/myproject/node_modules -> /workspace/node_modules (rw, workspace)
  - bind /home/user/myproject/package-lock.json -> /workspace/package-lock.json (rw, workspace)
  - mask /workspace/.npmrc (credential masked)
```

---

## Run your first sandboxed command

```bash
sbox run -- npm install
```

The first run pulls the container image if it's not already local (a few seconds to a few minutes depending on image size). Subsequent runs use the cached image.

npm runs inside the container. Postinstall scripts can only reach `registry.npmjs.org` — arbitrary internet hosts are blocked. They cannot read your SSH keys or tokens, cannot write outside `node_modules/` and `package-lock.json`. When the container exits, `node_modules/` is on your host as usual.

---

## What's next

**If it worked:** read [Progressive adoption](adoption.md) to understand the residual risk from Stage 1 and how to close it.

**If something broke:** check [Troubleshooting](troubleshooting.md). The most common issues are read-only filesystem errors (fix with `writable_paths`) and missing env vars (fix with `pass_through`).

**If you want to understand what the sandbox actually does:** read [How it works](how-it-works.md).

**If you need to download packages from the registry:** `network: off` blocks downloads too. Read [Network security](network.md) for the options.
