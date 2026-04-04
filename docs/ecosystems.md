# Ecosystem Guides

Worked examples for common language ecosystems.

---

## Node.js (npm / pnpm / yarn / bun)

### Zero-config setup

```yaml
version: 1

workspace:
  mount: /workspace
  writable: false
  exclude_paths:
    - .env
    - .env.local
    - .npmrc
    - .netrc
    - ".ssh/*"
    - ".aws/*"

image:
  ref: node:22-bookworm-slim

environment:
  pass_through:
    - TERM

package_manager:
  name: npm   # or: yarn | pnpm | bun
```

That's the entire config. sbox automatically:
- Routes `npm install` → network on, only `registry.npmjs.org` reachable, `node_modules` and `package-lock.json` writable
- Routes `npm run build` → network off, only `dist` writable, postinstall code can't touch source
- Routes everything else → network off, workspace fully read-only

### With a persistent cache (faster re-installs)

```yaml
package_manager:
  name: npm

environment:
  set:
    npm_config_cache: /var/tmp/sbox/npm-cache

caches:
  - name: npm-cache
    target: /var/tmp/sbox/npm-cache
```

### With a pre-install audit

```yaml
package_manager:
  name: npm
  pre_run:
    - npm audit --audit-level=high
```

### Advanced: full manual config

If you need more control than `package_manager:` provides, use explicit profiles:

```yaml
profiles:
  install:
    mode: sandbox
    network: on
    network_allow:
      - "*.npmjs.org"
    writable: false
    writable_paths:
      - node_modules
      - package-lock.json
    role: install
    no_new_privileges: true
    lockfile_files:
      - package-lock.json
      - npm-shrinkwrap.json
    pre_run:
      - npm audit --audit-level=high

  build:
    mode: sandbox
    network: off
    writable: false
    writable_paths:
      - dist
      - node_modules/.vite-temp
      - node_modules/.tmp
    no_new_privileges: true

  default:
    mode: sandbox
    network: off
    writable: false
    writable_paths: []
    no_new_privileges: true

dispatch:
  npm-install:
    match:
      - "npm install*"
      - "npm ci"
    profile: install
  npm-build:
    match:
      - "npm run build*"
    profile: build
```

### pnpm / yarn / bun

Same zero-config approach — just change the name:

```yaml
package_manager:
  name: pnpm   # or yarn, bun
```

For a persistent cache:

| Manager | env var | cache path |
|---------|---------|-----------|
| pnpm | `PNPM_HOME` | `/var/tmp/sbox/pnpm-store` |
| yarn | `YARN_CACHE_FOLDER` | `/var/tmp/sbox/yarn-cache` |
| bun | `BUN_INSTALL_CACHE_DIR` | `/var/tmp/sbox/bun-cache` |

---

## Python (uv / pip / poetry)

### Zero-config setup

```yaml
image:
  ref: python:3.13-slim

package_manager:
  name: uv   # or: pip | poetry
```

Routes `uv sync` / `uv add` → network on, `pypi.org` only, `.venv` writable. Everything else → network off.

### With cache

```yaml
package_manager:
  name: uv

environment:
  set:
    UV_CACHE_DIR: /var/tmp/sbox/uv-cache

caches:
  - name: uv-cache
    target: /var/tmp/sbox/uv-cache
```

For poetry, use `POETRY_CACHE_DIR` and `POETRY_VIRTUALENVS_IN_PROJECT: "true"`. For pip, use `PIP_CACHE_DIR`.

---

## Rust (cargo)

```yaml
image:
  ref: rust:1-bookworm

package_manager:
  name: cargo
```

Routes `cargo build` / `cargo fetch` → network on, `crates.io` only, `target` writable. Release builds → network off.

With a persistent registry cache:

```yaml
package_manager:
  name: cargo

environment:
  set:
    CARGO_HOME: /var/tmp/sbox/cargo-home

caches:
  - name: cargo-registry
    target: /var/tmp/sbox/cargo-home
```

---

## Go

```yaml
image:
  ref: golang:1.23-bookworm

package_manager:
  name: go
```

Routes `go get` / `go mod download` → network on, `proxy.golang.org` only, `vendor` writable. Builds → network off.

With a module cache:

```yaml
package_manager:
  name: go

environment:
  set:
    GOPATH: /var/tmp/sbox/go
    GOPROXY: https://proxy.golang.org,direct

caches:
  - name: go-mod-cache
    target: /var/tmp/sbox/go
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
