# Progressive Adoption

sbox does not require you to rewrite your workflow on day one. You can adopt it in stages, each adding more protection without breaking how you work.

---

## Stage 0 — Inspect without changing anything

`sbox plan` shows exactly what would happen if you ran a command in a sandbox. No container is started. Nothing changes.

```bash
cd myproject
sbox init --preset node    # create a sbox.yaml in seconds
sbox plan -- npm install   # see what the sandbox would look like
```

You get a full view of: which image, which mounts, what environment variables are filtered, what network policy applies, which profile was selected. Zero workflow disruption.

This alone is useful: it makes your install policy **visible and reviewable**, even if you never run the sandbox.

---

## Stage 1 — Sandbox one command

Pick the highest-risk command — typically `npm install` or `pip install` — and run it through sbox. Keep everything else on the host.

```bash
sbox run -- npm install    # sandboxed
npm run build              # still on host
node server.js             # still on host
```

If something breaks, `sbox plan -- npm install` shows you exactly what changed. The most common issue is a file the install script expected to write that's now read-only — fix it with `writable_paths`.

---

## Stage 2 — Add shims for transparent interception

Once the sandboxed install works, install shims so you don't have to type `sbox run --` every time:

```bash
sbox shim
export PATH="$HOME/.local/bin:$PATH"
```

Now `npm install` in any project with `sbox.yaml` automatically uses the sandbox. In projects without `sbox.yaml`, npm runs normally — the shim is invisible.

Your existing scripts, Makefiles, and CI configs don't need to change.

---

## Stage 3 — Extend to build and run commands

Add profiles and dispatch rules for build and run commands:

```yaml
profiles:
  default:
    mode: sandbox
    network: off
    writable: true
    no_new_privileges: true

dispatch:
  npm-install:
    match:
      - "npm install*"
    profile: install
  npm-run:
    match:
      - "npm run*"
    profile: default
```

Now `npm run build`, `npm run test`, etc. also run in a sandbox with network off.

---

## Stage 4 — Enable strict mode in CI

Once the sandbox is stable in local dev, enforce stronger guarantees in CI:

```yaml
runtime:
  strict_security: true
  require_pinned_image: true
```

Strict mode refuses if:
- the image is not pinned to a digest
- a lockfile is missing for install-style commands
- sensitive env vars are passed through

This catches configuration drift before it reaches production.

---

## Common friction points and fixes

### `EROFS: read-only file system`

The install script tried to write to the workspace root. Add the file to `writable_paths`:

```yaml
workspace:
  writable_paths:
    - node_modules
    - package-lock.json   # add this if npm needs to update it
```

Or use `--no-save` for one-off installs.

### Command not found inside the container

The image doesn't have the tool you're trying to run. Either install it in the image or use a different base image:

```yaml
image:
  ref: node:22-bookworm-slim   # includes node, npm, npx
```

### Environment variable missing inside the container

Only vars in `pass_through` are forwarded. Add the missing one:

```yaml
environment:
  pass_through:
    - TERM
    - MY_NEEDED_VAR
```

### Install needs the internet but `network: off` is set

Use `network_allow` to allow only the registry:

```yaml
profiles:
  install:
    network: on
    network_allow:
      - "*.npmjs.org"
```

See [network.md](network.md) for details.

---

## Team rollout

1. Add `sbox.yaml` to the repo and commit it — this is the opt-in signal
2. Document `sbox shim` in your project README or onboarding guide
3. Add `sbox run -- npm ci` to CI as a parallel check first; don't replace the existing install until it's stable
4. Once CI is green, replace the native install with the sandboxed one
5. Enable `strict_security: true` in CI after a stabilization period
