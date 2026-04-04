# Ecosystem Guides

Worked examples for common language ecosystems. Each shows a production-ready `sbox.yaml` with caching, credential masking, and install policy.

---

## Node.js (npm / pnpm / yarn / bun)

### Recommended config

```yaml
version: 1

runtime:
  backend: podman
  rootless: true

workspace:
  root: .
  mount: /workspace
  writable: false
  writable_paths:
    - node_modules
  exclude_paths:
    - .env
    - .env.local
    - .npmrc
    - .netrc
    - ".ssh/*"
    - ".aws/*"

image:
  ref: node:22-bookworm-slim
  digest: sha256:<pin this>

environment:
  pass_through:
    - TERM
    - CI
  set:
    npm_config_cache: /var/tmp/sbox/npm-cache
  deny:
    - NPM_TOKEN
    - NODE_AUTH_TOKEN

caches:
  - name: npm-cache
    target: /var/tmp/sbox/npm-cache

profiles:
  install:
    mode: sandbox
    network: on
    network_allow:
      - "*.npmjs.org"
    writable: true
    role: install
    no_new_privileges: true
    lockfile_files:
      - package-lock.json
      - npm-shrinkwrap.json
    pre_run:
      - npm audit --audit-level=high

  default:
    mode: sandbox
    network: off
    writable: true
    no_new_privileges: true

dispatch:
  npm-install:
    match:
      - "npm install*"
      - "npm ci"
    profile: install
  npm-run:
    match:
      - "npm run*"
      - "npx *"
    profile: default
```

### Key decisions

- `network_allow: ["*.npmjs.org"]` — allows download from the npm registry; blocks exfiltration to arbitrary hosts
- `npm_config_cache` — redirects the npm cache into a persistent volume so the package cache survives between runs
- `NPM_TOKEN` is in `deny` — even if set in the host environment, it never reaches the container
- `pre_run: npm audit` — runs an audit on the host before allowing the sandboxed install
- `lockfile_files` — in strict mode, refuses install if `package-lock.json` doesn't exist yet

### pnpm

Add to `environment.set`:

```yaml
set:
  PNPM_HOME: /var/tmp/sbox/pnpm-store
```

Add a cache:

```yaml
caches:
  - name: pnpm-store
    target: /var/tmp/sbox/pnpm-store
```

Update dispatch:

```yaml
dispatch:
  pnpm-install:
    match:
      - "pnpm install*"
      - "pnpm add*"
    profile: install
```

### yarn (classic)

```yaml
environment:
  set:
    YARN_CACHE_FOLDER: /var/tmp/sbox/yarn-cache

caches:
  - name: yarn-cache
    target: /var/tmp/sbox/yarn-cache
```

### bun

```yaml
environment:
  set:
    BUN_INSTALL_CACHE_DIR: /var/tmp/sbox/bun-cache

caches:
  - name: bun-cache
    target: /var/tmp/sbox/bun-cache

profiles:
  install:
    mode: sandbox
    network: on
    network_allow:
      - "*.npmjs.org"
      - registry.npmjs.org
    role: install
    lockfile_files:
      - bun.lockb
      - bun.lock
```

---

## Python (uv / pip / poetry)

### uv (recommended)

```yaml
version: 1

runtime:
  backend: podman
  rootless: true

workspace:
  root: .
  mount: /workspace
  writable: false
  writable_paths:
    - .venv
  exclude_paths:
    - .env
    - .env.local
    - ".ssh/*"
    - ".aws/*"

image:
  ref: python:3.13-slim
  digest: sha256:<pin this>

environment:
  pass_through:
    - TERM
    - CI
  set:
    UV_CACHE_DIR: /var/tmp/sbox/uv-cache

caches:
  - name: uv-cache
    target: /var/tmp/sbox/uv-cache

profiles:
  install:
    mode: sandbox
    network: on
    network_allow:
      - "*.pypi.org"
      - files.pythonhosted.org
    writable: true
    role: install
    no_new_privileges: true
    lockfile_files:
      - uv.lock

  default:
    mode: sandbox
    network: off
    writable: true
    no_new_privileges: true

dispatch:
  uv-sync:
    match:
      - "uv sync*"
      - "uv add*"
    profile: install
  uv-run:
    match:
      - "uv run*"
    profile: default
```

### poetry

```yaml
environment:
  set:
    POETRY_CACHE_DIR: /var/tmp/sbox/poetry-cache
    POETRY_VIRTUALENVS_IN_PROJECT: "true"

caches:
  - name: poetry-cache
    target: /var/tmp/sbox/poetry-cache

profiles:
  install:
    mode: sandbox
    network: on
    network_allow:
      - "*.pypi.org"
      - files.pythonhosted.org
    role: install
    lockfile_files:
      - poetry.lock

dispatch:
  poetry-install:
    match:
      - "poetry install*"
      - "poetry add*"
    profile: install
```

### pip

```yaml
environment:
  set:
    PIP_CACHE_DIR: /var/tmp/sbox/pip-cache

caches:
  - name: pip-cache
    target: /var/tmp/sbox/pip-cache

profiles:
  install:
    mode: sandbox
    network: on
    network_allow:
      - "*.pypi.org"
      - files.pythonhosted.org
    role: install

dispatch:
  pip-install:
    match:
      - "pip install*"
      - "pip3 install*"
    profile: install
```

---

## Rust (cargo)

```yaml
version: 1

runtime:
  backend: podman
  rootless: true

workspace:
  root: .
  mount: /workspace
  writable: false
  writable_paths:
    - target
  exclude_paths:
    - ".ssh/*"
    - ".aws/*"

image:
  ref: rust:1-bookworm
  digest: sha256:<pin this>

environment:
  pass_through:
    - TERM
    - CI
  set:
    CARGO_HOME: /var/tmp/sbox/cargo-home
    CARGO_TARGET_DIR: /workspace/target

caches:
  - name: cargo-registry
    target: /var/tmp/sbox/cargo-home

profiles:
  build:
    mode: sandbox
    network: on
    network_allow:
      - "*.crates.io"
      - static.crates.io
      - "*.github.com"
    writable: true
    role: install
    no_new_privileges: true
    lockfile_files:
      - Cargo.lock

  test:
    mode: sandbox
    network: off
    writable: true
    no_new_privileges: true

dispatch:
  cargo-build:
    match:
      - "cargo build*"
      - "cargo check*"
      - "cargo fetch*"
    profile: build
  cargo-test:
    match:
      - "cargo test*"
    profile: test
```

---

## Go

```yaml
version: 1

runtime:
  backend: podman
  rootless: true

workspace:
  root: .
  mount: /workspace
  writable: false
  writable_paths:
    - vendor
  exclude_paths:
    - ".ssh/*"
    - ".aws/*"

image:
  ref: golang:1.23-bookworm
  digest: sha256:<pin this>

environment:
  pass_through:
    - TERM
    - CI
  set:
    GOPATH: /var/tmp/sbox/go
    GOPROXY: https://proxy.golang.org,direct
    GONOSUMCHECK: ""

caches:
  - name: go-mod-cache
    target: /var/tmp/sbox/go

profiles:
  download:
    mode: sandbox
    network: on
    network_allow:
      - proxy.golang.org
      - sum.golang.org
      - "*.pkg.go.dev"
    writable: true
    role: install
    no_new_privileges: true
    lockfile_files:
      - go.sum

  build:
    mode: sandbox
    network: off
    writable: true
    no_new_privileges: true

dispatch:
  go-mod-download:
    match:
      - "go mod download*"
      - "go get*"
    profile: download
  go-build:
    match:
      - "go build*"
      - "go test*"
    profile: build
```

---

## Getting the image digest

Pin the image digest to prevent unexpected image changes:

```bash
podman pull node:22-bookworm-slim
podman inspect node:22-bookworm-slim --format '{{index .RepoDigests 0}}'
# docker.io/library/node@sha256:abc123...
```

Then set in `sbox.yaml`:

```yaml
image:
  ref: node:22-bookworm-slim
  digest: sha256:abc123...
```
