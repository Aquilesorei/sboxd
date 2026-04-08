# Getting Started

## What you need

- Linux, macOS, or Windows
- Rootless Podman **or** Docker installed and working
- That's it

If you're not sure whether your setup is correct, run `sbox doctor` after installing — it checks everything and tells you what to fix.

---

## Install sbox

**macOS — Homebrew (recommended):**

```bash
brew tap aquilesorei/sbox
brew install sbox
```

**Linux — pre-built binary:**

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

**macOS — pre-built binary:**

```bash
# Apple Silicon
curl -fsSL https://github.com/Aquilesorei/sboxd/releases/latest/download/sbox-macos-aarch64 \
  -o ~/.local/bin/sbox && chmod +x ~/.local/bin/sbox

# Intel
curl -fsSL https://github.com/Aquilesorei/sboxd/releases/latest/download/sbox-macos-x86_64 \
  -o ~/.local/bin/sbox && chmod +x ~/.local/bin/sbox
```

**Windows — PowerShell:**

```powershell
Invoke-WebRequest -Uri https://github.com/Aquilesorei/sboxd/releases/latest/download/sbox-windows-x86_64.exe `
  -OutFile "$env:USERPROFILE\.local\bin\sbox.exe"
$env:PATH += ";$env:USERPROFILE\.local\bin"
```

**From crates.io (any platform):**

```bash
cargo install sboxd
```

**From source:**

```bash
git clone https://github.com/Aquilesorei/sboxd
cd sboxd
cargo install --path .
```

---

## Shell completions

```bash
sbox completions bash  >> ~/.bash_completion
sbox completions zsh   >  ~/.zsh/completions/_sbox
sbox completions fish  >  ~/.config/fish/completions/sbox.fish
```

---

## Check everything works

```bash
sbox doctor
```

> **Tip:** Many `sbox` commands have single-letter shortcuts (e.g., `sbox d` for `sbox doctor`, `sbox r` for `sbox run`). See the [Usage Guide](usage.md) for a full list.

Checks backend availability, rootless mode, signature verification support, and shim health. Fix anything it flags before continuing.

---

## Zero-Config Mode (Shadow Infrastructure)

If your project already has a `Dockerfile` or a Docker Compose file (`docker-compose.yml`, `compose.yaml`), you can use `sbox` immediately without creating an `sbox.yaml` file.

```bash
# In a project with a Dockerfile
sbox run -- npm install

# In a project with a docker-compose.yml
sbox run --service app -- npm install
```

When no `sbox.yaml` is present, `sbox` enters **Shadow Mode**:
1. **Case A (Compose)**: It detects your Compose file, identifies the primary service (or use `--service`), and runs your command in a hardened container sharing the same network as your sidecars.
2. **Case B (Dockerfile)**: It builds a temporary image from your `Dockerfile` and runs your command inside it.
3. **Case C (Fallback)**: If no infrastructure is found, it uses a default secure `ubuntu:24.04` image with network access disabled by default.

This allows you to get the security benefits of `sbox` on any project with zero setup.

---

## Add sbox to a project (Explicit Config)

### Option 1 — smart auto-detect (fastest)

In most projects, plain `sbox init` is enough. It scans the repo and adapts to what already exists:
- lockfiles and manifests such as `package.json`, `pyproject.toml`, `Cargo.toml`, and `go.mod`
- `Dockerfile` or `Containerfile`
- Compose files such as `docker-compose.yml`, `compose.yaml`, or `podman-compose.yml`

```bash
cd myproject
sbox init
```

Examples:
- `package-lock.json` → npm
- `uv.lock` → uv
- `Cargo.toml` / `Cargo.lock` → cargo
- `go.mod` / `go.sum` → go
- `Dockerfile` / `Containerfile` → uses the existing build definition for `image.build`
- Compose files → imports sidecars and infers backend preference

`sbox init --from-lockfile` still exists if you want lockfile-only detection.

### Option 2 — named preset

```bash
sbox init --preset node       # Node.js — npm, node:22-bookworm-slim
sbox init --preset python     # Python  — uv,  ghcr.io/astral-sh/uv:python3.13-bookworm-slim
sbox init --preset rust       # Rust    — cargo, rust:1-bookworm
sbox init --preset go         # Go      — go, golang:1.23-bookworm
sbox init --preset generic    # Blank   — ubuntu:24.04, manual profiles
```

### Option 3 — interactive wizard

```bash
cd myproject
sbox init --interactive
sbox init --interactive --all
```

The wizard has two behaviors:

**Simple mode** (recommended for most projects):

1. Setup mode → `simple`
2. Package manager
3. Container image
4. Container backend
5. Optional command aliases

What changes in practice:
- if the repo already fixes the package manager, `sbox` uses it and skips that prompt
- if the repo already has `Dockerfile` / `Containerfile` or Compose image info, `sbox` uses it and skips or narrows the image/backend questions
- where a field is optional, the prompt includes a `skip` path

Writes a minimal config with `package_manager:`. sbox infers install profiles, build profiles, network policy, and writable paths from the preset.

**Advanced mode** (for custom policies):

1. Setup mode → `advanced`
2. Container backend
3. Language / ecosystem
4. Container image
5. Default network access
6. Writable paths
7. Whether to add dispatch rules
8. Optional command aliases

Writes a config with explicit `profiles:` and `dispatch:` for full manual control.

**`--all` mode**:
- `sbox init --interactive --all`
- starts from a blank interactive flow
- does not auto-apply detected defaults from the repo
- useful when you want complete manual control even in a repo with existing Docker/Podman files

Press Enter at any prompt to accept the default.

---

## See what will happen before running anything

```bash
sbox plan -- npm install           # show resolved policy
sbox plan --audit -- npm install   # show policy + run npm audit inline
sbox --output-format json plan -- npm install  # render as machine-readable JSON
sbox run --dry-run -- npm install  # show policy + the exact backend command, no execution
```

`sbox plan` resolves the full execution policy and prints it — which image, which mounts, which env vars pass through, what network policy — without starting a container. Use this to understand and debug your config.

Pass `--audit` to also run the ecosystem's native audit tool (`npm audit`, `cargo audit`, `pip-audit`, etc.) and append findings to the plan output.

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
