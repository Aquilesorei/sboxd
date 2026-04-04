# sbox Implementation Roadmap

## Objective

Build `sbox` as a Rust CLI that executes arbitrary project commands either on the host or inside a sandboxed container environment, with behavior resolved from configuration, profiles, dispatch rules, and safe defaults.

Status: v1 and v2 are both complete.

## Scope For v1 — Complete

- Config loading from `sbox.yaml`
- CLI surface for `init`, `run`, `exec`, `shell`, `plan`, `doctor`, and `clean`
- Host and sandbox execution modes
- Profile resolution and dispatch matching
- Workspace mount and cwd mapping
- Environment variable policy
- Extra mounts, caches, and secrets
- Podman backend
- Exit code and streaming behavior

## Spec Issues Resolved

1. Section numbering inconsistency — treated as implementation-level only; spec not updated.
2. Ports-with-network-off example — validator rejects this combination.
3. Strict validation ambiguity — resolved: strict mode is an explicit opt-in flag.
4. `status` and `logs` — deferred to post-v2; not implemented.
5. Rebuild detection for `image.build` — simple tag-based check; no content hashing in v1/v2.
6. Cache scoping — workspace-root-hashed volume names; one predictable rule.

## Architecture

Modules as implemented:

```text
src/
  main.rs
  cli.rs
  app.rs
  error.rs
  config/
    mod.rs
    model.rs
    load.rs
    validate.rs
  dispatch.rs
  plan.rs
  resolve.rs
  exec.rs
  shim.rs
  shell.rs
  doctor.rs
  clean.rs
  init.rs
  backend/
    mod.rs
    podman.rs
    docker.rs
```

Core design rules followed:
- CLI parsing is separate from execution logic
- All behavior resolves into a single `ExecutionPlan` before any execution
- Backend-specific command generation is an adapter boundary (`backend/podman.rs`, `backend/docker.rs`)
- Config validation and runtime validation are separate passes
- Host and sandbox execution share the same plan model

## v1 Delivery Phases — All Complete

### Phase 0: Bootstrap — Complete
### Phase 1: CLI And Config Loading — Complete
### Phase 2: Resolution Engine — Complete
### Phase 3: Host Execution — Complete
### Phase 4: Podman Sandbox Backend — Complete
### Phase 5: `init`, `doctor`, And `clean` — Complete
### Phase 6: Reuse And Interactive Shells — Complete
### Phase 7: Hardening And Compatibility — Complete

## Definition Of Done For v1 — Met

A Linux user can:
- initialize a project config
- inspect the execution plan for any command
- run arbitrary commands on host or in a Podman sandbox
- rely on profile and dispatch-based policy selection
- use mounts, caches, env vars, and secrets intentionally
- get predictable exit codes, output streaming, and working-directory behavior
- understand failures without reading the source

---

## v2 Delivery Phases — All Complete

### Phase 8: Trust And Verification — Complete

Implemented:
- `runtime.require_pinned_image: true` — global image trust enforcement across all sandbox profiles
- Per-profile `require_pinned_image: true` — profile-level digest requirement
- Config-load-time validation: if `require_pinned_image` is set but no digest is configured, sbox fails before execution with an actionable message
- `trusted_image_required` in `ExecutionAudit` reflects both profile-level and runtime-level flags
- Image trust level (`pinned-digest`, `mutable-reference`, `local-build`) visible in `sbox plan`
- Real signature verification via `skopeo` and containers policy (`verify_signature: true` on image config)
- `doctor` reports whether signature verification is usable on the current machine

### Phase 9: Automated Integration Coverage — Complete

Implemented:
- Gated real-Podman integration tests (`SBOX_RUN_PODMAN_TESTS=1`): workspace/cwd behavior, network-off enforcement, reusable sessions, cleanup, port mapping
- Golden tests for representative `sbox plan` output: `npm`, `uv`, `bun`, `poetry`
- Opt-in signature-verification integration test (`SBOX_SIGNATURE_POLICY` + `SBOX_SIGNED_TEST_IMAGE`)
- 213 unit and integration tests passing

### Phase 10: Package-Manager-Agnostic Security Hooks — Complete

Implemented (replacing earlier PM-specific approach):
- `role: install | run | build` on profiles — declares install-style semantics without hardcoding package manager names
- `lockfile_files: [...]` — per-profile list of lockfile filenames to check for presence
- `pre_run: [...]` — list of host-side commands run before the sandboxed command; failure aborts execution
- `require_lockfile: true` — refuses install-style commands in strict mode when lockfile is absent
- `sbox plan` audit section shows: `install_style`, `lockfile` state, `pre_run` commands
- Examples updated: `npm`, `uv`, `bun`, `poetry` all use the agnostic model

Previously implemented PM-specific items (`script_policy`, `audit_hooks`) have been replaced by the above.

### Phase 11: Compatibility Without Trust Regression — Complete

Implemented:
- Docker backend fully functional: `build_run_args`, reusable sessions, `exec`, `shell`
- Per-profile image overrides: `profile.image` accepts `ref`, `build`, `preset`, `digest`, `verify_signature`
- Backend auto-detection: `runtime.backend` is now optional; sbox probes PATH for `podman` then `docker` when not specified
- `doctor` uses the same auto-detection logic for backend health checks
- Transparent shim interception (`sbox shim`) for `npm`, `pnpm`, `yarn`, `bun`, `uv`, `pip`, `pip3`, `poetry`, `cargo`, `composer`
- Outbound network domain allow-listing (`network_allow`) with three supported entry forms:
  - Exact hostname (`registry.npmjs.org`) — DNS-resolved to IPs, injected as `--add-host`
  - Glob pattern (`*.npmjs.org`) — base domain resolved, pattern stored for display
  - Regex-prefix pattern (`.*\.npmjs\.org`) — base domain unescaped and resolved
- `sbox plan` shows resolved entries and raw patterns separately

### Phase 12: Stronger Isolation Backends — Deferred

Decision: not implemented in v2. The rootless Podman model (no-new-privileges, read-only rootfs, network-off, credential masking, env filtering) is sufficient for the stated threat model — malicious postinstall scripts. A microVM or gVisor backend would only add value if the threat model expands to "code that breaks out of a Linux container namespace," which is a different attacker profile. Deferred to a future track if that need emerges.

## Definition Of Done For v2 — Met

A Linux user can:
- enforce global or per-profile image trust rules with config-load-time validation
- rely on automated coverage for the main Podman security paths
- use package-manager-agnostic install policy (`role`, `pre_run`, `lockfile_files`) without hardcoding PM names
- restrict outbound DNS to an explicit allow-list with glob/regex pattern support
- use either Podman or Docker with auto-detection when backend is not specified
- intercept package manager invocations transparently via shims

---

## Remaining Work (Post-v2)

The following items are out of scope for v1/v2 but worth tracking for a future track:

- **`status` and `logs` subcommands** — monitor running reusable sessions
- **Rebuild detection for `image.build`** — hash the Dockerfile and relevant build context to detect when a rebuild is needed
- **Remote runner mode** — execute plans on a remote host or in CI without a local container runtime
- **Non-Linux platform support** — macOS (Lima/Podman Machine) or Windows (WSL2)
- **Microvm/gVisor backend** — stronger isolation for higher-risk workloads if the threat model expands
- **Wildcard DNS enforcement for unknown domains** — for well-known package registry base domains (`npmjs.org`, `pypi.org`, `crates.io`, `github.com`, etc.) `*.example.org` patterns expand to the full set of known subdomains; for unknown base domains only the base itself is resolved. Full enforcement for arbitrary wildcards requires a container-side DNS proxy (e.g. CoreDNS or dnsmasq) which would need `CAP_NET_BIND_SERVICE` or a privilege escalation path — deferred
- **Per-secret profile conditions** — `when_profiles` filtering is implemented; richer conditional logic (e.g. `when_command_matches`) is not
