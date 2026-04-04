# Troubleshooting

## Diagnosis first: `sbox plan` and `sbox doctor`

Before debugging a failed run, always check:

```bash
sbox plan -- <your command>   # shows exactly what will be run and why
sbox doctor                   # checks backend health, signature verification, shim paths
```

---

## Container fails to start

### `lsetxattr /dev/null: operation not permitted`

**Cause:** Rootless Podman + SELinux enforcing on Fedora/RHEL. Occurs when Podman tries to apply SELinux labels to device nodes inside the container.

**Fix:** sbox automatically passes `--security-opt label=disable` when using `keep-id` user mapping. If you see this error, make sure you are running sbox 0.1.1 or later.

```bash
sbox --version
```

### `Error: crun: ... permission denied`

**Cause:** The host directory being mounted has permissions that the container user cannot read.

**Fix:** Check ownership of the workspace:

```bash
ls -la .
```

With rootless Podman and `keep-id`, the container runs as your host user. If files are owned by root or another user, reads will fail.

### `Error: image not found`

**Cause:** The image isn't pulled yet, or `pull_policy: never` is set.

**Fix:** Pull manually:

```bash
podman pull node:22-bookworm-slim
```

Or remove `pull_policy: never` from your config.

### Image digest mismatch

**Cause:** `image.digest` is set but the local image doesn't match — the image was re-pulled and the digest changed.

**Fix:** Update the digest in `sbox.yaml`:

```bash
podman inspect node:22-bookworm-slim --format '{{index .RepoDigests 0}}'
```

---

## npm / package manager errors

### `EROFS: read-only file system`

**Cause:** npm is trying to write to a file not in `writable_paths`.

Common cases:
- `package.json` or `package-lock.json` — npm wants to update them during install
- `.npmrc` in the workspace — excluded by `exclude_paths`, so the `/dev/null` mask is read-only

**Fix:** Add the path to `writable_paths`, or pass `--no-save` to npm:

```bash
sbox run -- npm install --no-save package-name
```

Or add `package-lock.json` to `writable_paths` if you want it updated:

```yaml
workspace:
  writable_paths:
    - node_modules
    - package-lock.json
```

### `ENOENT: no such file or directory` for a package tarball

**Cause:** You passed a host path to npm instead of the container path. The workspace is mounted at `/workspace`, not at its host path.

**Fix:** Use the container path:

```bash
# Wrong:
sbox run -- npm install /tmp/mypackage-1.0.0.tgz

# Right (if the tgz is in the workspace root):
sbox run -- npm install /workspace/mypackage-1.0.0.tgz
```

### npm output is empty / postinstall results missing

**Cause:** npm suppresses stdout/stderr from lifecycle scripts when they exit with code 0. Only failure output is shown.

**Fix:** If you need script output, write it to a file in `node_modules` (the only writable path) and read it from the host after the install. See the [adversarial test suite](../tests/adversarial/) for an example of this pattern.

---

## Network errors

### `ENETUNREACH` or `connection refused` during install

**Cause:** The install profile has `network: off` but the package manager needs the internet to download packages.

**Fix:** Use `network: on` with `network_allow` to allow only the registry:

```yaml
profiles:
  install:
    network: on
    network_allow:
      - "*.npmjs.org"
```

See [network.md](network.md) for details.

### DNS resolution fails for an allowed domain

**Cause:** `network_allow` resolves hostnames at plan time and injects IPs into `--add-host`. If the IP changes between plan and run (CDN rotation), the injected address may be stale.

**Fix:** Add the specific IP manually, or accept that `network_allow` is best-effort for CDN-backed registries. For critical CI pipelines, use a fixed private registry with a stable IP.

### `network_allow` has no effect

**Cause:** `network: off` is set in the same profile. `network_allow` only applies when `network: on`.

**Fix:**

```yaml
profiles:
  install:
    network: on          # must be on for network_allow to work
    network_allow:
      - registry.npmjs.org
```

---

## Credential / environment issues

### Token is present in the container even though it's in `deny`

**Cause:** The token is in `environment.set` AND `environment.deny`. In sbox 0.1.0 and earlier, `set` bypassed `deny`. Fixed in 0.1.1.

**Fix:** Upgrade to sbox 0.1.1 or later. `deny` now takes precedence over `set`.

### Host env var is not reaching the container

**Cause:** Only vars listed in `pass_through` are forwarded. Everything else is stripped.

**Fix:** Add the variable to `pass_through`:

```yaml
environment:
  pass_through:
    - MY_VAR
```

Or inject it explicitly with `set`:

```yaml
environment:
  set:
    MY_VAR: my-value
```

---

## Config errors

### `sbox.yaml` not found

**Cause:** sbox walks up from the current directory looking for `sbox.yaml`. If none is found, it errors.

**Fix:** Run `sbox init --interactive` in your project root, or specify the config explicitly:

```bash
sbox --config /path/to/sbox.yaml run -- npm install
```

### `unknown profile`

**Cause:** `sbox exec <profile> -- <command>` references a profile that doesn't exist in `sbox.yaml`.

**Fix:** Check available profiles with:

```bash
sbox plan -- <command>
```

The `profile source` line shows which profile was selected.

### Strict mode refusing execution

```
error: strict security: image is not pinned to a digest
error: strict security: lockfile not found
error: strict security: sensitive variable passed through
```

**Fix:** Address each complaint:
- Pin the image: `image.digest: sha256:...`
- Add the lockfile or run `npm install` first to generate it
- Remove the sensitive var from `pass_through` or add it to `deny`

---

## Shim issues

### Shim is not intercepting commands

**Cause:** The real binary appears before the shim directory in `PATH`.

**Fix:** Make sure `~/.local/bin` (or wherever shims are installed) comes before the real binary:

```bash
export PATH="$HOME/.local/bin:$PATH"
which npm   # should show ~/.local/bin/npm
```

### Shim runs on a project without `sbox.yaml`

**Cause:** This is expected — shims fall through to the real binary when no `sbox.yaml` is found. No sandbox is applied.

---

## Performance

### Container startup is slow

Rootless Podman adds ~0.5–1s of startup overhead per invocation. For repeated commands, enable reusable sessions:

```yaml
runtime:
  reuse_container: true
```

Or at the profile level:

```yaml
profiles:
  dev:
    reuse_container: true
```

A named container is started once and reused for all subsequent `sbox run` calls in that workspace. Clean it up with:

```bash
sbox clean
```

### Image pull is slow on first run

Pull the image separately before running:

```bash
podman pull node:22-bookworm-slim
```

Subsequent runs use the local image without pulling.
