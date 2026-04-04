# Progressive Adoption

You do not need to sandbox everything on day one. sbox is designed to be added to an existing project incrementally — one command at a time — without breaking your current workflow.

---

## Why incremental matters

The biggest reason teams don't adopt sandboxing tools is friction. If adopting a tool means rewriting your Makefile, changing your CI pipeline, and debugging five broken environment variables before anything works, it won't happen — especially right after an incident when you're already under pressure.

sbox is built so you can start with zero risk and add containment gradually as you understand the behavior.

---

## Stage 0 — Look before you touch anything

Start with `sbox plan`. It shows exactly what a sandboxed command would look like without running it. No container starts. Nothing changes on disk.

```bash
cd myproject
sbox init --preset node      # generates sbox.yaml in seconds
sbox plan -- npm install     # shows the resolved policy
```

The output tells you: which container image, which directories are mounted and how, which environment variables are filtered, what network policy applies, and which profile was selected. You can review this like a diff before committing to anything.

This step alone has value. It makes your install policy **visible and reviewable**. You can add `sbox.yaml` to source control and treat it as a security policy document, even before you run a single sandboxed command.

---

## Stage 1 — Sandbox the install step

The highest-risk command is `npm install` (or `pip install`, `cargo build`, etc.) — that is where postinstall scripts run. Sandbox that one command. Leave everything else alone.

```bash
sbox run -- npm install    # sandboxed: postinstall scripts cannot reach host
npm run build              # still on host
node server.js             # still on host
```

This works because `node_modules/` is a bind mount. When the container writes packages to `/workspace/node_modules/`, those files land on your host filesystem at `./node_modules/`. When the container exits, the files stay. `npm run build` on the host finds them exactly where it expects.

If something breaks at this stage, `sbox plan -- npm install` shows you why. The most common issue is a file the install script tried to write that is now read-only — fix it with `writable_paths`:

```yaml
workspace:
  writable_paths:
    - node_modules
    - package-lock.json   # add this if npm needs to update it
```

**The residual risk at this stage:** whatever landed in `node_modules/` runs on your host with your full privileges when you call `npm run build`. The sandbox contained the postinstall scripts during install. It did not contain what was installed. A malicious package could plant code in `node_modules/.bin/` that runs during your next build step. Stage 3 closes this gap.

---

## Stage 2 — Make it transparent with shims

Once the sandboxed install works, stop typing `sbox run --` manually. Install shims:

```bash
sbox shim
export PATH="$HOME/.local/bin:$PATH"   # add to ~/.bashrc or ~/.zshrc
```

Now `npm install` in any project that has `sbox.yaml` automatically runs through sbox. In projects without `sbox.yaml`, npm runs exactly as before — the shim is a no-op.

Your Makefile, your CI scripts, your muscle memory — none of it needs to change. The sandbox activates by presence of `sbox.yaml`, not by changing how you invoke commands.

---

## Stage 3 — Extend to build and run commands

This is where you close the residual risk from Stage 1. Add profiles and dispatch rules for your build and run commands:

```yaml
profiles:
  install:
    mode: sandbox
    network: on
    network_allow:
      - "*.npmjs.org"
    writable: true
    role: install
    no_new_privileges: true

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
  everything-else:
    match:
      - "npm run*"
      - "npx *"
      - "node *"
    profile: default
```

Now `npm run build`, `npx eslint`, and `node server.js` also run in a sandbox. The code in `node_modules/` never executes directly on your host — it always runs inside a container with `network: off`.

With shims installed (Stage 2), you still just type `npm run build`. The sandbox is invisible.

---

## Stage 4 — Lock it down in CI

Once the sandbox is stable in local development, enforce stronger guarantees in CI where the risk is highest:

```yaml
runtime:
  strict_security: true
  require_pinned_image: true
```

**What strict mode adds:**
- Refuses if the image is not pinned to a digest — prevents silent image drift
- Refuses if a lockfile is missing for install commands — enforces reproducibility
- Refuses if sensitive env vars are passed through — no accidental `NPM_TOKEN` leakage

Pin the image digest:

```bash
podman pull node:22-bookworm-slim
podman inspect node:22-bookworm-slim --format '{{index .RepoDigests 0}}'
# docker.io/library/node@sha256:abc123...
```

```yaml
image:
  ref: node:22-bookworm-slim
  digest: sha256:abc123...
```

Now even if someone pushes a malicious update to `node:22-bookworm-slim`, your CI runs the pinned version you reviewed.

---

## Common friction and fixes at each stage

### Stage 1 friction

**`EROFS: read-only file system`** — npm tried to write to a path not in `writable_paths`. Add it:
```yaml
workspace:
  writable_paths:
    - node_modules
    - package-lock.json
```

**`ENOENT` for a package tarball** — you passed a host path to npm but the container sees it at a different path. The workspace is mounted at `/workspace`, not at its host path. Use the container path:
```bash
sbox run -- npm install /workspace/mypackage-1.0.0.tgz
```

**Environment variable missing** — only vars in `pass_through` reach the container. Add what you need:
```yaml
environment:
  pass_through:
    - TERM
    - CI
    - MY_NEEDED_VAR
```

### Stage 2 friction

**Shim runs real npm instead of sbox** — the real binary is earlier in PATH. Check:
```bash
which npm   # must show ~/.local/bin/npm, not /usr/bin/npm
```
Make sure `export PATH="$HOME/.local/bin:$PATH"` is in your shell profile and your terminal reloaded it.

### Stage 3 friction

**Container startup is slow for build commands** — each `sbox run` starts a fresh container (~0.5–1s). For build/run commands where you want faster iteration, enable reuse:
```yaml
profiles:
  default:
    reuse_container: true
```
The container stays alive between calls. Use `sbox clean` to reset it.

**Tool not found inside container** — your build script calls something that isn't in the image. Either add it to the image or switch to a fuller base image:
```yaml
image:
  ref: node:22-bookworm   # full Debian, more tools available
```
