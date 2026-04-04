# Architecture

Internal reference for contributors.

## Execution flow

```
CLI (main.rs + cli.rs)
  └─ app::run()
       ├─ init / plan / doctor / clean / shim  →  direct execution
       └─ run / exec / shell
            1. load_config()               config/load.rs  — find and parse sbox.yaml
            2. resolve_execution_plan()    resolve.rs      — merge config + profile → ExecutionPlan
            3. validate_execution_safety() config/validate.rs  — security policy checks
            4. run_audit_hooks()           exec.rs         — pre_run commands on host
            5. execute_plan()
                 ├─ host mode  → direct process spawn
                 └─ sandbox    → backend/podman.rs or backend/docker.rs
```

Every `sbox run` follows this exact path. There are no shortcuts.

## Key modules

### `resolve.rs` — the policy engine

The most important file. Takes a parsed `Config` and a selected `ProfileConfig` and produces a single `ExecutionPlan` that captures everything needed to execute the command.

Key responsibilities:
- Merge runtime defaults with profile overrides
- Resolve workspace mounts and `writable_paths` into `ResolvedMount` entries
- Apply `exclude_paths`: walk the workspace, match files, add mask mounts
- Resolve environment: apply `pass_through`, `set`, `deny` filtering
- Resolve network: turn `network_allow` entries into `(hostname, ip)` pairs
- Resolve image: pin to digest, determine trust level
- Resolve user: `keep-id`, explicit uid/gid, or default
- Detect install-style commands and fill `ExecutionAudit`

Nothing outside `resolve.rs` and `backend/*.rs` should build command arguments.

### `config/model.rs` — the data model

All `serde` structs for `sbox.yaml`. If you add a new config key, it starts here.

### `config/validate.rs` — config-time validation

Runs after parsing but before any execution. Catches configuration errors early with actionable messages. Examples: `network_allow` + `network: off`, `require_pinned_image` without a digest, dangerous mount sources.

### `exec.rs` — orchestration

Calls `resolve`, `validate`, audit hooks, then dispatches to host or sandbox execution. Also handles `--strict-security` enforcement.

### `backend/podman.rs` and `backend/docker.rs`

Translate an `ExecutionPlan` into a `podman run` or `docker run` invocation. The only place where container runtime arguments are constructed.

Key functions:
- `build_run_args_with_options()` — constructs the full argument list for `run`
- `append_mount_args()` — translates `ResolvedMount` entries into `--mount` flags
- `append_container_settings()` — applies security options, user mapping, capabilities
- `ensure_reusable_container()` — manages named container lifecycle for `reuse_container: true`

### `dispatch.rs`

Matches the command string against dispatch rules using simple glob patterns. Returns the matching profile name or `None`.

### `plan.rs`

Renders an `ExecutionPlan` as human-readable text for `sbox plan`. No logic here — just display.

### `init.rs`

Implements `sbox init`. Both the preset-based template generator and the `--interactive` wizard (using `dialoguer`).

### `doctor.rs`

Health checks: backend availability, rootless mode, `skopeo` availability, signature policy usability, shim paths.

### `shim.rs`

Generates shell wrapper scripts for package managers. Each shim checks for `sbox.yaml` and delegates to `sbox run` or falls through to the real binary.

## Data flow

```
sbox.yaml (file)
    │
    ▼
Config (config/model.rs)
    │  parsed by serde_yaml
    ▼
ProfileConfig (selected by dispatch or --profile)
    │
    ▼
ExecutionPlan (resolve.rs)
    │  contains: image, mounts, env, network policy, user mapping, audit data
    ▼
[validate_execution_safety]
    │
    ▼
[run_audit_hooks (pre_run on host)]
    │
    ▼
podman/docker run args (backend/*.rs)
    │
    ▼
container process
```

## ExecutionPlan

The central data structure. Everything needed to run the command — no raw config fields.

```rust
pub struct ExecutionPlan {
    pub command: Vec<String>,
    pub workspace: ResolvedWorkspace,
    pub image: ResolvedImageSource,
    pub policy: ResolvedPolicy,
    pub mounts: Vec<ResolvedMount>,
    pub caches: Vec<ResolvedCache>,
    pub secrets: Vec<ResolvedSecret>,
    pub environment: ResolvedEnvironment,
    pub user: ResolvedUser,
    pub audit: ExecutionAudit,
    pub profile_name: String,
    pub profile_source: ProfileSource,
    pub mode: ExecutionMode,
}
```

`sbox plan` renders this struct. `backend/*.rs` consumes it to build container arguments.

## Adding a new config field

1. Add the field to the appropriate struct in `config/model.rs`
2. Add validation in `config/validate.rs` if needed
3. Add resolution logic in `resolve.rs` — populate the `ExecutionPlan`
4. Add display in `plan.rs`
5. Consume in `backend/podman.rs` (and `docker.rs` if applicable)
6. Add a test

## Adding a new subcommand

1. Add a `FooCommand` struct to `cli.rs`
2. Add it to the `Commands` enum
3. Add `foo.rs` with an `execute()` function
4. Wire it in `app.rs`

## Testing

```bash
# All tests (no Podman required)
cargo test

# Unit tests only
cargo test --lib

# Specific test file
cargo test --test resolve_tests
cargo test --test validate_tests
cargo test --test plan_golden_tests

# Integration tests (requires working rootless Podman)
SBOX_RUN_PODMAN_TESTS=1 cargo test --test podman_integration_tests
```

### Golden tests

`tests/plan_golden_tests.rs` compares `sbox plan` output against `.txt` files in `tests/golden/`. When plan output changes intentionally:

```bash
# Update all golden files
UPDATE_GOLDEN=1 cargo test --test plan_golden_tests
```

### Adversarial tests

See [adversarial-testing.md](adversarial-testing.md).

## Error handling

All errors are typed via `thiserror` in `error.rs`. Every function returns `Result<_, SboxError>`. No `unwrap()` in production paths.

Adding a new error: add a variant to `SboxError` in `error.rs` with a `#[error("...")]` message.

## Security invariants

- `deny` always overrides `pass_through` and `set` in environment resolution
- mask mounts (exclude_paths) are always added after the workspace mount so they take precedence
- `--security-opt label=disable` is always added with `keep-id` user mapping (SELinux compatibility)
- `--security-opt no-new-privileges` is the default in sandbox profiles
- the home directory is never mounted silently
- dangerous sources (container sockets, absolute paths to credential dirs) are rejected in validation
