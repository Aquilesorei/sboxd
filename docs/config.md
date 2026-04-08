# sbox.yaml Reference

## Minimal example

The shortest working config uses `package_manager:` to generate everything automatically:

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

That's it. sbox infers the install profile (network on, registry-only allow-list, lockfile writable), the build profile (network off, dist writable), and a locked-down default for everything else.

For full control, use explicit `profiles:` and `dispatch:` instead — described below.

---

## `runtime`

```yaml
runtime:
  backend: podman           # podman | docker | omit for auto-detect
  rootless: true            # rootless mode (recommended)
  reuse_container: false    # keep container running between sbox invocations
  strict_security: false    # refuse execution if security policy is too loose
  require_pinned_image: false  # require image.digest globally
  network_policy: dns           # dns | firewall (Linux rootful only)
  compose:                      # sidecar services from Docker Compose
    file: docker-compose.yml
    services: [db, redis]
```

| Field | Default | Description |
|-------|---------|-------------|
| `backend` | auto | `podman` or `docker`. Omit to auto-detect from PATH (Podman preferred). |
| `rootless` | `true` | Run containers as the current user, not root. |
| `reuse_container` | `false` | Keep a named container running; use `podman exec` for subsequent invocations. |
| `strict_security` | `false` | Refuse if sensitive env vars are passed through, lockfile missing, or image unpinned. |
| `require_pinned_image` | `false` | Globally require `image.digest` to be set. |
| `network_policy` | `dns` | `dns` (hostname filtering) or `firewall` (strict IP-level egress). |
| `compose` | none | Sidecar services to start before execution. |

---

## `workspace`

```yaml
workspace:
  root: .                   # host directory to mount (default: current dir)
  mount: /workspace         # container path
  writable: false           # mount entire workspace read-only
  writable_paths:           # punch writable holes
    - node_modules
    - .venv
  exclude_paths:            # mask these files with /dev/null inside the container
    - .env
    - .env.local
    - "*.pem"
    - "*.key"
    - .npmrc
    - .netrc
    - ".ssh/*"
    - ".aws/*"
    - ".docker/*"
    - ".kube/*"
```

| Field | Description |
|-------|-------------|
| `root` | Host path to mount. Relative paths are resolved from the config file location. |
| `mount` | Container path where the workspace is mounted. |
| `writable` | If `false`, the entire workspace is read-only except for `writable_paths`. |
| `writable_paths` | List of workspace-relative paths that are mounted read-write even when `writable: false`. File entries are supported; sbox makes the parent directory writable so lockfile rewrites still work. |
| `exclude_paths` | Files matching these patterns are replaced with an empty `/dev/null` bind mount inside the container. Supports glob patterns (`*.key`) and directory globs (`.ssh/*`). |

---

## `image`

```yaml
image:
  ref: node:22-bookworm-slim
  build: Dockerfile             # build image from Dockerfile if missing or changed
  digest: sha256:abc123...          # optional: pin to exact digest
  verify_signature: false           # verify via skopeo + containers policy
  pull_policy: missing              # missing | always | never
```

| Field | Description |
|-------|-------------|
| `ref` | Image reference. Passed directly to the container runtime. |
| `build` | Path to a `Dockerfile` (relative to workspace root). If the image is missing or the `Dockerfile` has a newer timestamp than the local image, `sbox` will rebuild it automatically. |
| `digest` | If set, the image reference is rewritten to `ref@digest` before pulling. Enforced at resolve time. |
| `verify_signature` | If `true`, runs `skopeo` against the system containers policy before execution. Requires a policy that actually enforces signatures. |
| `pull_policy` | `missing` (default): pull only if not present locally. `always`: always pull. `never`: fail if not present. |

---

## `environment`

```yaml
environment:
  pass_through:             # host vars to forward into the container
    - TERM
    - CI
  set:                      # inject these values explicitly
    NODE_ENV: production
  deny:                     # block these vars (overrides pass_through and set)
    - NPM_TOKEN
    - NODE_AUTH_TOKEN
    - AWS_SECRET_ACCESS_KEY
    - AWS_ACCESS_KEY_ID
    - GITHUB_TOKEN
```

- `deny` takes precedence over both `pass_through` and `set`.
- Variables not in `pass_through` are not forwarded even if present on the host.
- In `strict_security` mode, sbox refuses if any `pass_through` variable looks sensitive (token, secret, key, password, credential).

---

## `package_manager`

The zero-config shortcut. Declare your package manager name and sbox generates all profiles and dispatch rules automatically.

```yaml
package_manager:
  name: npm          # npm | yarn | pnpm | bun | uv | pip | poetry | cargo | go
```

What gets generated for `name: npm`:

| Generated profile | Network | Writable paths | Role |
|-------------------|---------|---------------|------|
| `pm-npm-install` | on + `registry.npmjs.org` only | `node_modules`, `package-lock.json` | install |
| `pm-npm-build` | off | `dist`, `node_modules/.vite-temp`, `node_modules/.tmp` | build |
| `default` | off | nothing | run |

Dispatch rules are prepended before any user-defined rules, so user dispatch always wins.

**Override fields:**

```yaml
package_manager:
  name: npm
  install_writable:        # replace preset install writable paths
    - node_modules
    - package-lock.json
    - .cache
  build_writable:          # replace preset build writable paths
    - dist
    - .nuxt
  network_allow:           # replace preset registry domains
    - registry.npmjs.org
    - my-private-registry.example.com
  pre_run:                 # host commands run before install (audit, lint, etc.)
    - npm audit --audit-level=high
```

All override fields are optional. Omit them to use the preset defaults.

**Supported presets:**

| name | install writable | build writable | registry domains |
|------|-----------------|----------------|-----------------|
| `npm` | `node_modules`, `package-lock.json` | `dist` | `registry.npmjs.org` |
| `yarn` | `node_modules`, `yarn.lock` | `dist` | `registry.yarnpkg.com` |
| `pnpm` | `node_modules`, `pnpm-lock.yaml` | `dist` | `registry.npmjs.org` |
| `bun` | `node_modules`, `bun.lockb` | `dist` | `registry.npmjs.org` |
| `uv` | `.venv` | `dist` | `pypi.org` |
| `pip` | `.venv` | — | `pypi.org` |
| `poetry` | `.venv` | `dist` | `pypi.org` |
| `cargo` | `target` | `target/release` | `crates.io` |
| `go` | `vendor` | — | `proxy.golang.org` |

`package_manager:` and `profiles:`/`dispatch:` can coexist. Explicit profiles take precedence.

---

## `profiles`

Each profile defines an execution policy. The `default` profile is used when no dispatch rule matches.

```yaml
profiles:
  install:
    mode: sandbox             # sandbox | host
    network: off              # off | on
    network_policy: dns       # dns | firewall (Linux rootful only)
    network_allow:            # hostname allowlist (only with network: on)
      - registry.npmjs.org
      - "*.npmjs.org"
    writable: true            # override workspace.writable for this profile
    no_new_privileges: true   # prevent privilege escalation
    read_only_rootfs: false   # make the container rootfs read-only
    role: install             # install | run | build — enables role-specific policy
    require_pinned_image: false
    compose:                  # sidecar services to start for this profile
      services: [db]
    lockfile_files:           # files that must exist for role: install in strict mode
      - package-lock.json
      - npm-shrinkwrap.json
    pre_run:                  # host commands run before the sandboxed command
      - npm audit --audit-level=high
    capabilities:             # Linux capabilities (three forms accepted)
      drop: [all]             #   { drop: [...], add: [...] } — structured (preferred)
      add: [NET_BIND_SERVICE] #   "drop-all"                 — drop all caps, add none
                              #   ["CAP_NET_ADMIN"]          — list treated as cap_add
    ports: []                 # published ports (only with network: on)
    reuse_container: false    # override runtime.reuse_container for this profile
    writable_paths:           # override workspace.writable_paths for this profile only
      - node_modules
      - package-lock.json
```

| Field | Default | Description |
|-------|---------|-------------|
| `mode` | `sandbox` | `sandbox` runs in a container. `host` runs the command directly on the host (no sandboxing). |
| `network` | `off` | `off` passes `--network none`. `on` uses the default container network. |
| `network_policy` | `dns` | `dns` (hostname filtering) or `firewall` (strict IP-level egress). `firewall` requires Linux + rootful mode. |
| `network_allow` | `[]` | Hostname allowlist. Requires `network: on`. See [network.md](network.md). |
| `writable` | inherited | If `true`, the workspace is mounted read-write. If unset, inherits from `workspace.writable`. |
| `no_new_privileges` | `true` | Passes `--security-opt no-new-privileges`. |
| `read_only_rootfs` | `false` | Passes `--read-only` to make the container filesystem read-only. |
| `role` | none | `install` enables lockfile checks and `pre_run` in strict mode. |
| `require_pinned_image` | `false` | Require `image.digest` for this profile. |
| `lockfile_files` | `[]` | Files to check for presence when `role: install` and strict mode is on. |
| `pre_run` | `[]` | Shell commands run on the **host** before the sandbox. If any fails, execution is aborted. |
| `capabilities` | none | Linux capabilities. Accepts three forms: `"drop-all"` (string), `["CAP_NET_ADMIN"]` (list treated as cap_add), or `{ drop: [...], add: [...] }` (structured, preferred). |
| `ports` | `[]` | Port mappings in `host:container` format. Only with `network: on`. |
| `writable_paths` | inherited | When set, overrides `workspace.writable_paths` for this profile only. Use this to give different profiles different write access (e.g. install writes lockfile, build writes dist). |

---

## `commands`

Custom command aliases for common development tasks. Defines a `sbox <alias>` command.

```yaml
commands:
  build:
    run: ["cargo", "build", "--release"]
    profile: build
    description: "Build the release binary"

  test:
    run: ["cargo", "test"]
    profile: test
    description: "Run the test suite"

  dev:
    run: ["npm", "run", "dev"]
    # profile omitted: uses dispatch rules
```

| Field | Description |
|-------|-------------|
| `run` | The command and arguments to execute (array of strings). |
| `profile` | (Optional) Profile to use. If omitted, standard dispatch rules apply. |
| `description` | (Optional) A short text shown in `sbox help`. |

When running a custom command (e.g., `sbox build --verbose`), extra arguments are automatically appended to the `run` list.

---

## `dispatch`

Routes commands to profiles automatically based on pattern matching.

```yaml
dispatch:
  npm-install:
    match:
      - "npm install*"
      - "npm ci"
    profile: install

  cargo-build:
    match:
      - "cargo build*"
      - "cargo check*"
    profile: build
```

- Patterns use simple glob matching: `*` matches any sequence of characters within the command string.
- The first matching rule wins.
- If no rule matches, the `default` profile is used (or the profile specified with `--profile`).

---

## `mounts`

Extra bind or tmpfs mounts beyond the workspace.

```yaml
mounts:
  - source: /run/user/1000/podman/podman.sock
    target: /var/run/docker.sock
    read_only: true
  - type: tmpfs
    target: /tmp
```

---

## `caches`

Named persistent volumes for package manager caches.

```yaml
caches:
  - name: npm-cache
    target: /root/.npm
  - name: pip-cache
    target: /root/.cache/pip
```

Cache volumes are named per-workspace (based on a hash of the workspace root path) so they are not shared between projects.

---

## `secrets`

Host files mounted read-only into the container.

```yaml
secrets:
  - source: ~/.pypirc
    target: /root/.pypirc
```

Secrets are mounted as read-only bind mounts. Unlike `environment.set`, they are files rather than env vars.

---

## `identity`

Override the container user/group mapping.

```yaml
identity:
  uid: 1000
  gid: 1000
  map_user: true    # equivalent to --userns=keep-id
```

When `runtime.rootless: true` and no `identity` is set, sbox uses `keep-id` by default, mapping the host user into the container so file ownership in `writable_paths` works correctly.
