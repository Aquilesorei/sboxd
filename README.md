# sbox

`sbox` is a policy-driven command runner that executes development commands inside a rootless Podman or Docker sandbox.

The threat it addresses: **malicious postinstall scripts**. Code that runs automatically during `npm install`, `pip install`, or `cargo build` can read `~/.ssh/id_rsa`, dump `AWS_SECRET_ACCESS_KEY` from the environment, or exfiltrate your source to an attacker's server. sbox runs those commands in an isolated container with no access to host credentials, no network unless you explicitly allow it, and a read-only workspace.

The March 31 2026 Axios npm supply chain attack (Sapphire Sleet RAT delivered via postinstall hook) is exactly the scenario sbox is built to contain. An `npm install` inside a sbox sandbox with `network: off` and credential masking would have blocked that payload entirely.

## Platform support

| Platform | Status |
|----------|--------|
| Linux + Podman | **First-class** — rootless, no root required |
| Linux + Docker | **Supported** — same feature set, requires Docker socket access |
| macOS + Docker Desktop | **Partial** — containers run in a Linux VM; most features work, `keep-id` user mapping may differ |
| macOS + Podman Machine | **Partial** — similar to Docker Desktop |
| Windows | **Not supported** |

## Status

v1 and v2 are both complete. Current implemented scope:

- `init`, `run`, `exec`, `shell`, `plan`, `doctor`, `clean`, and `shim`
- Podman and Docker backends for sandbox execution
- Reusable Podman/Docker sessions when enabled
- Security validation: dangerous mounts, sensitive env pass-through, lockfile checks
- `--strict-security` and `runtime.strict_security: true`
- Per-profile and global `require_pinned_image` enforcement
- Image digest pinning and real signature verification via `skopeo` + containers policy
- Package-manager-agnostic policy: `role`, `pre_run`, `lockfile_files` on profiles
- Outbound network domain allow-listing with glob/regex pattern support
- Backend auto-detection when `runtime.backend` is not set
- Transparent shim interception for `npm`, `pnpm`, `yarn`, `bun`, `uv`, `pip`, `poetry`, `cargo`, and more

## Installation

**From crates.io:**

```bash
cargo install sboxd
```

**Pre-built binaries** (Linux x86_64 and aarch64) are attached to each [GitHub Release](../../releases):

```bash
curl -fsSL https://github.com/Aquilesorei/sboxd/releases/latest/download/sbox-linux-x86_64 -o ~/.local/bin/sbox
chmod +x ~/.local/bin/sbox
```

**From source:**

```bash
git clone https://github.com/Aquilesorei/sboxd
cd sbox
cargo install --path .
```

## Documentation

- [Getting started](docs/getting-started.md)
- [Progressive adoption](docs/adoption.md) — how to add sbox to an existing project without breaking your workflow
- [Ecosystem guides](docs/ecosystems.md) — Node.js, Python, Rust, Go configs
- [Network security](docs/network.md) — `network: off` vs `network_allow`, the download/postinstall tension
- [Security model](docs/security.md) — what is blocked, what is not, adversarial test results
- [Shims](docs/shims.md) — transparent interception for npm, pip, cargo, and others
- [Recipes](docs/recipes.md) — CI pipelines, private registries, reusable sessions
- [Troubleshooting](docs/troubleshooting.md) — common errors and fixes
- [Config reference](docs/config.md) — full `sbox.yaml` field reference
- [Architecture](docs/architecture.md) — internals and contribution guide

## Quick Start

Initialize a project config interactively (arrow-key menus, no manual editing):

```bash
sbox init --interactive
```

Or generate from a preset directly:

```bash
sbox init --preset node    # node / python / rust / go / generic
```

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

Then add to your shell profile:

```bash
export PATH="$HOME/.local/bin:$PATH"
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

A minimal config:

```yaml
version: 1

runtime:
  backend: podman
  rootless: true

workspace:
  mount: /workspace
  writable: false

image:
  ref: node:22-bookworm-slim
  digest: sha256:...

profiles:
  default:
    mode: sandbox
    network: off

  install:
    mode: sandbox
    network: on
    role: install
    require_pinned_image: true
    lockfile_files:
      - package-lock.json
    pre_run:
      - npm audit --audit-level=high
    network_allow:
      - registry.npmjs.org

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
