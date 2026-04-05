# Changelog

All notable changes to sbox are documented here.

## [0.1.7] - 2026-04-05

### Added
- `sbox doctor` now checks whether Docker is running in rootless mode and prints a fix if not
- `sbox run --dry-run` — resolves the plan and prints it (including the backend command) without executing
- `sbox run -e NAME=VALUE` — inject extra environment variables into the sandbox at invocation time (repeatable)
- End-to-end preset smoke tests: `python` and `node` presets are loaded and fully resolved in CI
- macOS support: `current_uid_gid()` falls back to `id -u`/`id -g` when `/proc/self/status` is unavailable
- Windows support (Docker Desktop): host paths emitted to Docker arguments are normalised to forward-slash form; `USERPROFILE` is respected for `~/` expansion

### Fixed
- **Docker root ownership**: Docker backend now always injects `--user UID:GID` for `ResolvedUser::Default`, matching the Podman `--userns keep-id` behaviour — files written to bind-mounted workspace dirs are owned by the host user regardless of whether Docker is running in rootless mode. **Note:** images that rely on running as root inside the container (e.g. custom entrypoints that call `apt-get`) must opt out by setting `identity: { uid: 0, gid: 0 }` in their profile
- **`detect_compose_image` heuristic**: multi-service compose files are now parsed with service-name awareness; well-known app service names (`app`, `web`, `api`, `backend`, `server`, `frontend`) are preferred over the first non-sidecar image found

## [0.1.6] - 2026-04-05

### Added
- CI pipeline (`.github/workflows/ci.yml`) with three jobs: unit tests, Docker integration tests, lint
- Warning when `backend: docker` is used without `rootless: true` — explains the file ownership problem and cleanup command

### Fixed
- Applied `cargo fmt` and `cargo clippy --fix` across the entire codebase

## [0.1.5] - 2026-04-05

### Added
- Docker backend (`backend/docker.rs`) — full implementation parallel to Podman
- 7 Docker integration tests gated by `SBOX_RUN_DOCKER_TESTS=1`
- `sbox audit` command — delegates to `npm audit`, `cargo audit`, `pip-audit`, `govulncheck`
- `package_manager:` preset auto-injects image, workspace, and runtime defaults — minimal config is now just `version: 1` + `package_manager: name: <pm>`
- `sbox init --interactive` detects existing `Dockerfile` and `docker-compose.yml` and offers to use them
- `UV_PYTHON_DOWNLOADS: never` auto-injected for the `uv` preset
- `examples/my_docker_app` (Dockerfile + uv) and `examples/compose-app` (docker-compose + npm)

### Fixed
- `sbox init --interactive` YAML indentation bug (Rust `\` line continuation was stripping indentation)
- Docker bind-mount: missing paths with file extensions are pre-created as files instead of directories
- uv preset default image changed from `python:3.13-slim` to `ghcr.io/astral-sh/uv:python3.13-bookworm-slim`

## [0.1.4] - 2026-04-04

### Added
- `package_manager:` preset system — declare the package manager name and sbox synthesises install/build profiles, network policy, lockfile expectations, and writable paths
- Per-profile `writable_paths` override
- Supply-chain security: cargo and go presets with correct `network_allow` registries; metadata endpoint denylist (`metadata.google.internal`, etc.); secret role restrictions

## [0.1.3] - 2026-04-04

### Added
- `sbox init --interactive` wizard with arrow-key selection (simple and advanced modes)

## [0.1.1] - 2026-04-04

### Added
- Adversarial test suite
- SELinux `:z` relabeling for bind mounts
- `deny` takes precedence over `set` in env policy
- `.ssh/` and `.aws/` masking in workspace

### Fixed
- Install command in README (`cargo install sboxd`)

## [0.1.0] - initial release

- Rootless Podman sandbox for package manager commands
- `sbox run`, `sbox exec`, `sbox shell`, `sbox plan`, `sbox doctor`, `sbox clean`, `sbox shim`, `sbox bootstrap`
- Policy-driven config: profiles, dispatch, network, environment filtering, secrets, caches
- Golden test suite and integration tests
