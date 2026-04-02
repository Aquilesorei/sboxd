# sbox

`sbox` is a policy-driven command runner for executing development commands on the host or inside a Podman sandbox.

The intended use case is hostile-by-default dependency installation. Package-manager commands such as `npm install` or `uv sync` should run in an isolated container with explicit mounts, explicit environment policy, and no accidental access to host credentials.

The current Phase 10 policy layer understands install-oriented flows for:

- `npm`, `pnpm`, `yarn`, `bun`
- `uv`, `pip`, `poetry`
- `cargo`, `go`, `composer`

## Status

Current implemented scope:

- `init`, `run`, `exec`, `shell`, `plan`, `doctor`, and `clean`
- Podman backend for sandbox execution
- reusable Podman sessions when enabled
- security validation for dangerous mounts and sensitive env pass-through
- `--strict-security` and `runtime.strict_security: true`
- digest pinning and real image-signature verification checks through `skopeo` plus containers policy

Current limitations:

- Docker sandbox execution is not implemented
- `doctor` only performs backend checks for Podman
- this repo currently has no gated integration test suite for local Podman; live backend checks have been done manually

## Quick Start

Initialize a project config:

```bash
sbox init
```

This creates one user-facing config file:

```text
sbox.yaml
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

## Security Model

The default security direction is:

- prefer rootless Podman
- disable network in normal sandbox profiles
- only pass through host env vars explicitly
- do not mount the home directory silently
- reject dangerous bind mounts such as container sockets and common credential paths
- keep dependency state outside the host workspace unless the user explicitly chooses otherwise

Strict mode hardens this further:

```bash
sbox --strict-security run -- node --version
```

or:

```yaml
runtime:
  strict_security: true
```

In strict mode, sandbox execution is refused if sensitive host variables are being passed through.

For sensitive profiles, `sbox` also supports:

```yaml
image:
  ref: ghcr.io/astral-sh/uv:python3.13-bookworm-slim
  digest: sha256:...
  verify_signature: true
```

Important:

- `digest` pins the image reference and is enforced by `sbox`
- `verify_signature: true` is a real runtime check, not metadata
- verification requires `skopeo` and a containers policy that actually enforces signatures
- a policy using `insecureAcceptAnything` does not count as verification

`sbox doctor` now reports whether signature verification is usable on the current machine and fails if the config requests it but the machine cannot enforce it.

Package-manager-sensitive profiles can also add extra policy:

```yaml
profiles:
  install:
    mode: sandbox
    network: on
    require_pinned_image: true
    require_lockfile: true
    script_policy: ignore
    audit_hooks:
      - npm-audit
```

These controls mean:

- `require_lockfile: true` refuses install-style commands in strict mode unless the expected lockfile is present
- `script_policy: ignore` requires lifecycle scripts to be disabled, for example with `--ignore-scripts`
- `script_policy: block` refuses script-capable package-manager commands entirely
- `audit_hooks` runs explicit preflight commands such as `npm audit`, `cargo audit`, `bun audit`, `composer audit`, or `govulncheck` before the main command and fails closed if the hook fails

## Fedora / Podman Signature Setup

On Fedora, Podman commonly reads signature policy from:

- `~/.config/containers/policy.json`
- `/etc/containers/policy.json`

The workstation default is often too permissive for `sbox` signature enforcement. If `doctor` reports:

```text
WARN signature-verify not currently usable: policy /etc/containers/policy.json does not enforce signature verification
```

set up a user policy instead of changing the system-wide default immediately.

Example files are provided in:

- [examples/fedora-podman-signature-policy/README.md](/home/aquiles/RustroverProjects/sbox/examples/fedora-podman-signature-policy/README.md)
- [examples/fedora-podman-signature-policy/policy.json](/home/aquiles/RustroverProjects/sbox/examples/fedora-podman-signature-policy/policy.json)
- [examples/fedora-podman-signature-policy/registries.d/example.yaml](/home/aquiles/RustroverProjects/sbox/examples/fedora-podman-signature-policy/registries.d/example.yaml)

Basic Fedora flow:

```bash
mkdir -p ~/.config/containers
mkdir -p ~/.config/containers/registries.d
cp examples/fedora-podman-signature-policy/policy.json ~/.config/containers/policy.json
cp examples/fedora-podman-signature-policy/registries.d/example.yaml ~/.config/containers/registries.d/example.yaml
```

Then replace the placeholder registry scope, GPG key path, and lookaside URL with your real values and run:

```bash
sbox doctor
```

For `verify_signature: true` to work, all of these must be true:

- `skopeo` is installed
- the selected containers policy contains a real verification rule such as `signedBy` or `sigstoreSigned`
- the configured registry scope matches the image reference
- the referenced keys and signature storage are valid

For the gated signature-verification integration test, provide both:

```bash
export SBOX_RUN_PODMAN_TESTS=1
export SBOX_SIGNATURE_POLICY=/path/to/verification-capable-policy.json
export SBOX_SIGNED_TEST_IMAGE=registry.example.com/team/secure-images/app@sha256:...
cargo test --test podman_integration_tests -- --nocapture
```

The signed-image test skips automatically if those variables are not set.

## `sbox.yaml`

Normal users should create and maintain only one config file:

```text
sbox.yaml
```

That file defines:

- runtime backend
- image
- workspace mount
- environment policy
- caches
- secrets
- profiles
- dispatch rules

## Examples

Repository examples:

- [sbox.yaml](/home/aquiles/RustroverProjects/sbox/sbox.yaml): `uv`-based Python example with isolated cache and environment
- [examples/python-smoke/reuse-sbox.yaml](/home/aquiles/RustroverProjects/sbox/examples/python-smoke/reuse-sbox.yaml): reusable Python sandbox session example
- [examples/npm-smoke/sbox.yaml](/home/aquiles/RustroverProjects/sbox/examples/npm-smoke/sbox.yaml): npm example with isolated cache, install prefix, and artifact storage
- [examples/bun-smoke/sbox.yaml](/home/aquiles/RustroverProjects/sbox/examples/bun-smoke/sbox.yaml): bun example with lockfile-aware install policy and an audit hook
- [examples/poetry-smoke/sbox.yaml](/home/aquiles/RustroverProjects/sbox/examples/poetry-smoke/sbox.yaml): poetry example with isolated cache and virtualenv paths

These examples are designed so dependency installation happens inside the sandbox and dependency state does not land in the host workspace by default.
