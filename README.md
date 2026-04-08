# sbox

`sbox` is a policy-driven command runner that executes development commands inside a rootless Podman or Docker sandbox.

The threat it addresses: **malicious postinstall scripts**. Code that runs automatically during `npm install`, `pip install`, or `cargo build` can read `~/.ssh/id_rsa`, dump `AWS_SECRET_ACCESS_KEY` from the environment, or exfiltrate your source to an attacker's server. sbox runs those commands in an isolated container with no access to host credentials, no network unless you explicitly allow it, and a read-only workspace.

The March 31 2026 Axios npm supply chain attack (Sapphire Sleet RAT delivered via postinstall hook) is exactly the scenario sbox is built to contain. An `npm install` inside a sbox sandbox with `network: off` and credential masking would have blocked that payload entirely.

## Platform support

| Platform | Status |
|----------|--------|
| Linux + Podman | **First-class** — rootless, no root required |
| Linux + Docker | **Supported** — same feature set, requires Docker socket access |
| macOS + Docker Desktop | **Supported** — pre-built binary available; containers run in a Linux VM |
| macOS + Podman Machine | **Supported** — same as Docker Desktop path |
| Windows + Docker Desktop | **Supported** — pre-built binary available; requires Docker Desktop with WSL2 backend |

## Status

v1 and v2 are both complete. Current implemented scope:

- `init`, `run`, `exec`, `shell`, `plan`, `doctor`, `clean`, `status`, `logs`, `audit`, `bootstrap`, `explain`, `shim` (shortcuts: `i`, `r`, `e`, `sh`, `p`, `d`, `c`, `s`, `l`, `a`, `b`, `ex`)
- Podman and Docker backends for sandbox execution
- Reusable Podman/Docker sessions when enabled
- Security validation: dangerous mounts, sensitive env pass-through, lockfile checks
- `--strict-security` and `runtime.strict_security: true`
- Per-profile and global `require_pinned_image` enforcement
- Image digest pinning and real signature verification via `skopeo` + containers policy
- Package-manager-agnostic policy: `role`, `pre_run`, `lockfile_files` on profiles
- Outbound network domain allow-listing with glob/regex pattern support
- **IP-level Firewall mode** for strict egress enforcement (Linux rootful only)
- **Docker Compose integration** for managing sidecar dependencies (databases, etc.)
- **Smart mtime-based rebuilds** for `image.build` when Dockerfile changes
- **Built-in shortcuts**: Many commands have single-letter aliases (e.g., `sbox r` for `sbox run`, `sbox p` for `sbox plan`)
- **Custom command aliases** via `commands:` in `sbox.yaml` (e.g., `sbox build`, `sbox test`)
- **Shadow Infrastructure (Zero-config)**: Run `sbox` in projects with only a Dockerfile or Compose file; no `sbox.yaml` required
- **Backend auto-detection** and manual override via `--backend` or `SBOX_BACKEND`
- Transparent shim interception for `npm`, `pnpm`, `yarn`, `bun`, `uv`, `pip`, `poetry`, `cargo`, and more

## Installation

**Linux (x86\_64):**

```bash
curl -fsSL https://github.com/Aquilesorei/sboxd/releases/latest/download/sbox-linux-x86_64 -o ~/.local/bin/sbox
chmod +x ~/.local/bin/sbox
```

**Linux (aarch64):**

```bash
curl -fsSL https://github.com/Aquilesorei/sboxd/releases/latest/download/sbox-linux-aarch64 -o ~/.local/bin/sbox
chmod +x ~/.local/bin/sbox
```

**macOS (Apple Silicon):**

```bash
curl -fsSL https://github.com/Aquilesorei/sboxd/releases/latest/download/sbox-macos-aarch64 -o ~/.local/bin/sbox
chmod +x ~/.local/bin/sbox
```

**macOS (Intel):**

```bash
curl -fsSL https://github.com/Aquilesorei/sboxd/releases/latest/download/sbox-macos-x86_64 -o ~/.local/bin/sbox
chmod +x ~/.local/bin/sbox
```

**Windows (x86\_64) — PowerShell:**

```powershell
Invoke-WebRequest -Uri https://github.com/Aquilesorei/sboxd/releases/latest/download/sbox-windows-x86_64.exe `
  -OutFile "$env:USERPROFILE\.local\bin\sbox.exe"
```

Add `%USERPROFILE%\.local\bin` to your `PATH` if it isn't already.

**From crates.io (any platform with Rust installed):**

```bash
cargo install sboxd
```

**From source:**

```bash
git clone https://github.com/Aquilesorei/sboxd
cd sboxd
cargo install --path .
```

## Documentation

Start with the **[Comprehensive Usage Guide](docs/usage.md)** — it covers every command, configuration option, and security policy in one place.

For specific topics, see:
- [How it works](docs/how-it-works.md) — the mental model
- [Getting started](docs/getting-started.md) — quick install and first run
- [Progressive adoption](docs/adoption.md) — add sbox to an existing project one step at a time
- [Ecosystem guides](docs/ecosystems.md) — Node.js, Python, Rust, Go
- [Network security](docs/network.md) — `network: off`, `network_allow`, two-phase installs
- [Security model](docs/security.md) — what is blocked, what is not, adversarial test results
- [Shims](docs/shims.md) — transparent interception for npm, pip, cargo, and others
- [Recipes](docs/recipes.md) — CI pipelines, private registries, reusable sessions
- [Troubleshooting](docs/troubleshooting.md) — common errors and fixes
- [Config reference](docs/config.md) — full `sbox.yaml` field reference
- [Architecture](docs/architecture.md) — internals and contribution guide

## Quick Start

### Zero-Config (Shadow Mode)

If your project has a `Dockerfile` or `docker-compose.yml`, just run it:

```bash
sbox run -- npm install
sbox run --service app -- npm run build
```

### Explicit Config

Initialize `sbox` in your project. By default, it scans the repo first and adapts to what is already there: lockfiles and manifests, `Dockerfile` or `Containerfile`, and Compose files.

```bash
sbox init
```

If the project already clearly implies an ecosystem or backend, `sbox init` uses that instead of asking.

Or force a specific preset (skips auto-detection):

```bash
sbox init --preset node      # npm  — node:22-bookworm-slim
sbox init --preset python    # pip/uv — python:3.13-slim
sbox init --preset rust      # cargo — rust:1-bookworm
sbox init --preset go        # go   — golang:1.23-bookworm
sbox init --preset generic   # blank — ubuntu:24.04
```

Or use the interactive wizard:

```bash
sbox init --interactive
sbox init --interactive --all
```

- `sbox init --interactive`
  - scans first
  - skips irrelevant prompts when the repo already makes the choice obvious
  - offers `skip` where a value is optional
  - can prompt for `commands:` aliases before writing the file
- `sbox init --interactive --all`
  - prompts from scratch
  - does not auto-apply detected defaults
  - useful when you want to configure everything manually

Inspect the resolved policy before running anything:

```bash
sbox plan -- uv sync
```

Run a command in the resolved environment:

```bash
sbox run -- uv sync
```

Run a command against a specific profile:

```bash
sbox exec deps -- uv sync
```

Open an interactive shell:

```bash
sbox shell
```

Check backend and policy health:

```bash
sbox doctor
```

Remove current-workspace reusable sessions:

```bash
sbox clean
```

### Transparent interception with shims

`sbox shim` generates thin wrapper scripts for common package managers. When one of these wrappers is called from a directory that has an `sbox.yaml`, it transparently delegates to `sbox run`. Otherwise it calls the real binary unchanged.

```bash
# Install shims to ~/.local/bin (must appear before real binaries in PATH)
sbox shim

# Or specify a directory
sbox shim --dir ~/bin

# Preview without writing anything
sbox shim --dry-run
```

Then add to your shell profile (Unix/macOS):

```bash
export PATH="$HOME/.local/bin:$PATH"
```

On Windows (PowerShell), add the shim directory to your user PATH:

```powershell
[Environment]::SetEnvironmentVariable(
  "PATH", "$env:USERPROFILE\.local\bin;$env:PATH", "User")
```

After this, `npm install`, `uv sync`, `bun install`, etc. are automatically sandboxed in any project that has an `sbox.yaml`.

## Security Model

The default direction is:

- prefer rootless Podman
- network off in sandbox profiles by default
- only pass through host env vars explicitly configured
- reject dangerous bind mounts: container sockets, `.ssh`, `.aws`, `.kube`, `.npmrc`, and similar
- do not mount the home directory silently
- keep dependency state outside the host workspace unless the user explicitly opts in

### Strict mode

```bash
sbox --strict-security run -- node --version
```

or:

```yaml
runtime:
  strict_security: true
```

Strict mode refuses sandbox execution if:
- sensitive host variables are being passed through
- install-style commands run without a lockfile present
- the image is not pinned to a digest

### Image trust

Pin an image globally:

```yaml
image:
  ref: node:22-bookworm-slim
  digest: sha256:3efebb4f5f2952af4c86fe443a4e219129cc36f90e93d1ea2c4aa6cf65bdecf2
```

Require a pinned image for all sandbox profiles globally:

```yaml
runtime:
  require_pinned_image: true
```

Require it only for a specific profile:

```yaml
profiles:
  install:
    mode: sandbox
    require_pinned_image: true
```

When `require_pinned_image` is set, sbox enforces this at config-load time and refuses execution if the image has no digest. It also integrates with `skopeo` for real signature verification:

```yaml
image:
  ref: ghcr.io/astral-sh/uv:python3.13-bookworm-slim
  digest: sha256:...
  verify_signature: true
```

- `digest` pins the image reference and is enforced at resolve time
- `verify_signature: true` is a real runtime check via `skopeo` — not metadata
- verification requires a containers policy that actually enforces signatures; a policy using `insecureAcceptAnything` does not count

`sbox doctor` reports whether signature verification is usable on the current machine.

### Network allow-listing

Profiles with `network: on` can restrict outbound DNS to a specific set of domains:

```yaml
profiles:
  install:
    mode: sandbox
    network: on
    network_allow:
      - registry.npmjs.org
      - "*.npmjs.org"
      - ".*\\.yarnpkg\\.com"
```

Three entry forms are supported:

| Form | Example | Behavior |
|------|---------|----------|
| Exact hostname | `registry.npmjs.org` | DNS-resolved to IPs, injected as `--add-host` |
| Glob prefix | `*.npmjs.org` | Base domain `npmjs.org` resolved, pattern stored for display |
| Regex prefix | `.*\.npmjs\.org` | Same as glob — base domain unescaped and resolved |

Enforcement works by pointing the container's DNS at a non-routable address (`192.0.2.1`) so arbitrary lookups time out, while injecting resolved IPs directly into `/etc/hosts` via `--add-host`. Raw IP connections bypass this; package managers use domain names.

For glob/regex patterns, sbox expands the base domain to the full set of known subdomains before resolving. `*.npmjs.org` resolves `registry.npmjs.org`, `npmjs.org`, and `www.npmjs.org` — not just `npmjs.org` alone. Built-in expansion tables cover:

- npm/yarn: `npmjs.org`, `yarnpkg.com`
- Python: `pypi.org`, `pythonhosted.org`
- Rust: `crates.io`
- Go: `golang.org`, `go.dev`
- Ruby: `rubygems.org`
- Maven/Gradle: `maven.org`, `gradle.org`
- GitHub: `github.com`, `githubusercontent.com`
- OCI registries: `docker.io`, `ghcr.io`, `gcr.io`

For unknown base domains only the base itself is resolved.

`sbox plan` shows the resolved state:

```
  network_allow: [resolved] registry.npmjs.org, npmjs.org, www.npmjs.org; [patterns] *.npmjs.org
```

### Install-style policy

Profiles declare their role explicitly rather than relying on command-pattern detection:

```yaml
profiles:
  install:
    mode: sandbox
    role: install
    lockfile_files:
      - package-lock.json
      - npm-shrinkwrap.json
    pre_run:
      - npm audit --audit-level=high
    require_lockfile: true
    require_pinned_image: true
```

- `role: install` — marks this profile as install-style; enables lockfile auditing
- `lockfile_files` — which files to check for presence before running
- `pre_run` — shell commands run on the **host** before the sandboxed command; if any fails, execution is refused
- `require_lockfile: true` — refuses install-style commands in strict mode unless a lockfile is present

### Credential masking

Credentials that exist in the workspace can be masked from the container using `/dev/null` bind mounts:

```yaml
workspace:
  exclude_paths:
    - .env
    - .env.local
    - "*.pem"
    - "*.key"
    - .npmrc
    - .netrc
```

Each matched file is replaced with a read-only `/dev/null` mount inside the container. This prevents postinstall scripts from reading secrets that happen to live in the project directory.

## Backend selection

sbox uses Podman by default. To use Docker:

```yaml
runtime:
  backend: docker
```

When `runtime.backend` is omitted, sbox probes PATH at execution time: Podman is preferred if available, Docker otherwise.

## `sbox.yaml` reference

The shortest working config uses `package_manager:` to generate all profiles automatically:

```yaml
version: 1

workspace:
  mount: /workspace
  writable: false
  exclude_paths:
    - .env
    - .npmrc
    - ".ssh/*"
    - ".aws/*"

image:
  ref: node:22-bookworm-slim

package_manager:
  name: npm
```

sbox infers the rest: install profile (network on, registry only, lockfile writable), build profile (network off, dist writable), and a locked-down default for everything else. No profiles or dispatch rules to write.

When a `writable_paths` entry names a file such as `uv.lock` or `Cargo.lock`, sbox mounts the file's parent directory read-write so package managers can use normal delete/rename/create lockfile workflows inside an otherwise read-only workspace.

For full control, use explicit profiles:

```yaml
profiles:
  install:
    mode: sandbox
    network: on
    network_allow:
      - registry.npmjs.org
    writable: false
    writable_paths:
      - node_modules
      - package-lock.json
    role: install
    lockfile_files:
      - package-lock.json

  default:
    mode: sandbox
    network: off
    writable: false
    writable_paths: []

dispatch:
  npm-install:
    match:
      - npm install
      - npm ci
    profile: install
```

Top-level keys:

| Key | Description |
|-----|-------------|
| `version` | Must be `1` |
| `runtime` | Backend, rootless mode, reuse settings, image trust policy |
| `workspace` | Host root, container mount path, writable policy, credential exclusions |
| `image` | Image reference, digest, build recipe, signature policy |
| `identity` | User/group mapping |
| `environment` | Env var pass-through, explicit set, deny list |
| `mounts` | Extra bind or tmpfs mounts |
| `caches` | Named persistent volumes |
| `secrets` | Host files mounted read-only into the container |
| `profiles` | Execution policies indexed by name |
| `dispatch` | Command pattern → profile routing rules |
| `package_manager` | Zero-config shortcut — generates profiles and dispatch automatically from a preset |

## Examples

Repository examples:

- [sbox.yaml](sbox.yaml): `uv`-based Python example with isolated cache and environment
- [examples/python-smoke/reuse-sbox.yaml](examples/python-smoke/reuse-sbox.yaml): reusable Python sandbox session
- [examples/npm-smoke/sbox.yaml](examples/npm-smoke/sbox.yaml): npm with isolated cache, install prefix, artifact storage, and network allow-list
- [examples/bun-smoke/sbox.yaml](examples/bun-smoke/sbox.yaml): bun with lockfile-aware install policy and preflight audit
- [examples/poetry-smoke/sbox.yaml](examples/poetry-smoke/sbox.yaml): poetry with isolated cache and virtualenv paths

## Post-install artifact risk

`sbox` isolates the install step. Once the sandbox exits, installed artifacts (`node_modules`, `.venv`, built binaries) live on the host. Any subsequent invocation outside of `sbox` — running `node`, `npx`, `python`, a script from `node_modules/.bin`, etc. — executes that code with full host privileges.

**Option 1 — Route all execution through sbox**

Add a `default` profile with `network: off` and route every project command through it:

```bash
sbox run -- npm start
sbox run -- node server.js
```

**Option 2 — Keep dependencies out of the workspace entirely**

Redirect package manager output to cache volumes so nothing lands in the workspace:

```yaml
environment:
  set:
    npm_config_prefix: /var/tmp/sbox/npm-prefix

caches:
  - name: npm-prefix
    target: /var/tmp/sbox/npm-prefix
```

Equivalent patterns exist for uv (`UV_PROJECT_ENVIRONMENT`), poetry (`POETRY_VIRTUALENVS_PATH`), and bun (`BUN_INSTALL_CACHE_DIR`).

## Fedora / Podman Signature Setup

On Fedora, Podman reads signature policy from `~/.config/containers/policy.json` or `/etc/containers/policy.json`. The workstation default is too permissive for `sbox` signature enforcement.

Example files are in [examples/fedora-podman-signature-policy/](examples/fedora-podman-signature-policy/).

```bash
mkdir -p ~/.config/containers/registries.d
cp examples/fedora-podman-signature-policy/policy.json ~/.config/containers/policy.json
cp examples/fedora-podman-signature-policy/registries.d/example.yaml ~/.config/containers/registries.d/example.yaml
```

Replace the placeholder registry scope, GPG key path, and lookaside URL with real values, then run `sbox doctor` to verify.

## Contact

Achille Zongo — achillezongo07@gmail.com
