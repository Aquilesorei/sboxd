# Recipes

Common patterns for real-world sbox usage.

---

## CI pipeline: locked install with audit

Run `npm ci` in CI with strict policy: locked install, pre-flight audit, registry-only network, pinned image.

```yaml
version: 1

runtime:
  backend: podman
  rootless: true
  strict_security: true

workspace:
  root: .
  mount: /workspace
  writable: false
  writable_paths:
    - node_modules
  exclude_paths:
    - .npmrc
    - .netrc
    - ".ssh/*"
    - ".aws/*"

image:
  ref: node:22-bookworm-slim
  digest: sha256:<pinned>

environment:
  pass_through:
    - CI
    - TERM
  deny:
    - NPM_TOKEN
    - NODE_AUTH_TOKEN
    - GITHUB_TOKEN

caches:
  - name: npm-cache
    target: /root/.npm

profiles:
  install:
    mode: sandbox
    network: on
    network_allow:
      - "*.npmjs.org"
    writable: true
    role: install
    no_new_privileges: true
    require_pinned_image: true
    lockfile_files:
      - package-lock.json
    pre_run:
      - npm audit --audit-level=critical

dispatch:
  ci:
    match:
      - "npm ci"
    profile: install
```

Usage in CI:

```bash
sbox run -- npm ci
```

---

## Two-phase install: download then script execution

Download packages with network, run postinstall scripts without network.

```yaml
profiles:
  download:
    mode: sandbox
    network: on
    network_allow:
      - "*.npmjs.org"
    writable: true
    role: install
    no_new_privileges: true

  run-scripts:
    mode: sandbox
    network: off
    writable: true
    no_new_privileges: true

dispatch:
  npm-download:
    match:
      - "npm install --ignore-scripts*"
    profile: download

  npm-rebuild:
    match:
      - "npm rebuild*"
      - "npm run prepare*"
    profile: run-scripts
```

Usage:

```bash
sbox run -- npm install --ignore-scripts   # download only, network allowed
sbox run -- npm rebuild                    # run scripts, network off
```

---

## Keep packages out of the workspace

Redirect npm output to a cache volume so nothing is written to the project directory. Useful when the workspace is read-only or when you want clean installs every time.

```yaml
environment:
  set:
    npm_config_prefix: /var/tmp/sbox/npm-prefix
    npm_config_cache: /var/tmp/sbox/npm-cache

caches:
  - name: npm-prefix
    target: /var/tmp/sbox/npm-prefix
  - name: npm-cache
    target: /var/tmp/sbox/npm-cache

workspace:
  writable: false
  writable_paths: []   # nothing writable in the workspace
```

The installed binaries land in `/var/tmp/sbox/npm-prefix/bin` inside the container. To run them:

```bash
sbox run -- /var/tmp/sbox/npm-prefix/bin/my-tool
```

---

## Reusable dev session

Keep a container running between commands. Faster startup; useful for interactive development.

```yaml
runtime:
  reuse_container: true

profiles:
  default:
    mode: sandbox
    network: off
    writable: true
```

```bash
sbox run -- npm run build   # starts container, runs command, keeps container alive
sbox run -- npm run test    # reuses the same container
sbox clean                  # stops and removes the container
```

---

## Private registry with credentials

Mount an `.npmrc` with a token as a secret, without exposing it through the environment.

```yaml
secrets:
  - source: ~/.npmrc
    target: /root/.npmrc

workspace:
  exclude_paths:
    - .npmrc    # mask the workspace .npmrc (may not have credentials)

profiles:
  install:
    network: on
    network_allow:
      - npm.myprivateregistry.com
```

The secret is mounted read-only inside the container. The source file on the host is not modified.

---

## Route all execution through sbox

Prevent installed artifacts from running outside the sandbox. Add a `default` profile that sandboxes everything.

```yaml
profiles:
  default:
    mode: sandbox
    network: off
    writable: true
    no_new_privileges: true

dispatch:
  install:
    match:
      - "npm install*"
      - "npm ci"
    profile: install
  # Everything else goes through the default sandbox
```

```bash
sbox run -- node server.js       # sandboxed
sbox run -- npx eslint src/      # sandboxed
sbox run -- npm run build        # sandboxed
```

---

## Docker instead of Podman

```yaml
runtime:
  backend: docker
  rootless: false
```

With Docker, rootless mode typically requires extra setup. If running as a non-root user with Docker socket access, set `rootless: false` and ensure the socket is accessible.

---

## Pin image and verify signature

```bash
# Get the digest
podman pull node:22-bookworm-slim
podman inspect node:22-bookworm-slim --format '{{index .RepoDigests 0}}'
```

```yaml
image:
  ref: node:22-bookworm-slim
  digest: sha256:abc123...
  verify_signature: true
```

`verify_signature` uses `skopeo` and the system containers policy. Check readiness:

```bash
sbox doctor
```

---

## Transparent shims for a team

Install shims system-wide so every developer automatically sandboxes package manager commands:

```bash
sudo sbox shim --dir /usr/local/bin
```

Or per-user:

```bash
sbox shim --dir ~/.local/bin
# add to ~/.bashrc or ~/.zshrc:
export PATH="$HOME/.local/bin:$PATH"
```

Shims pass through to the real binary in directories without `sbox.yaml`. Only projects that have opted in (by having `sbox.yaml`) get sandboxed.

---

## Multiple images per profile

Override the image for a specific profile without changing the global image:

```yaml
image:
  ref: node:22-bookworm-slim

profiles:
  build:
    mode: sandbox
    image:
      ref: node:22-bookworm-slim
      digest: sha256:<pinned>

  lint:
    mode: sandbox
    image:
      ref: node:20-alpine    # different image for linting
```
