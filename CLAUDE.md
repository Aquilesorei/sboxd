# AI Agent Guidelines

This file provides guidance to AI coding assistants working in this repository.

## Architecture

`sbox` is a Linux-native sandbox designed to secure development workflows. It does not use Docker, Podman, or any OCI container runtime. It relies exclusively on Linux kernel primitives.

### Core Primitives

1. **Landlock LSM (ABI V4)**: 
   - Filesystem: Grants read-only access to system paths and read-write to the workspace. Blocks sensitive paths completely (`~/.ssh`, `~/.aws`).
   - Network: Uses `AccessNet::ConnectTcp` to blanket-deny outgoing network connections by default. Permits `BindTcp` to support local servers.

2. **Egress Proxy** (`src/proxy.rs`):
   - Landlock's network rules key on **port number only**, not destination host — `AccessNet::ConnectTcp` cannot express "allow api.stripe.com, block attacker.example.com" on its own. This is a real gap: a legitimate app that needs both `--allow-net-out` and `--allow-env` (e.g. a server fetching a DB URL from `.env`) forces both doors open at once for every dependency in its tree, including malicious ones.
   - Fix: `--allow-net-out=host1,host2` spawns a local HTTP/HTTPS forward proxy (`EgressProxy::spawn`, plain `std::net`, no new deps) bound to an ephemeral `127.0.0.1` port, *before* the Landlock ruleset is built. The Landlock rule then permits `ConnectTcp` to exactly that one port via `NetPort::new(port, AccessNet::ConnectTcp)` — nothing else. The child's `HTTPS_PROXY`/`HTTP_PROXY` env vars point at it.
   - The proxy is the thing that actually understands "host," so it's where the allowlist lives: `CONNECT` (HTTPS) and raw HTTP requests are inspected for their target host before the tunnel opens; non-allowlisted hosts get `403` and are logged to stderr.
   - **Known limitation**: since the Landlock rule is port-scoped, not address-scoped, a remote service coincidentally listening on the exact same ephemeral port as the proxy would technically also be reachable directly. The port is randomized per run specifically to make this collision astronomically unlikely, not to make it impossible — this is a documented ceiling, not a bug to chase.
   - `--allow-net-out` with no hosts still works for backward compatibility (blanket allow, no proxy, no enforcement) but prints a deprecation warning. Prefer the host-restricted form.
   - **Unsolved by design, not by oversight**: the proxy allowlists by *host*, not by *secret*. A command run with both `--allow-env` and `--allow-net-out=<host>` still lets any code in that process — including a malicious nested dependency — send `.env` secrets to that same allowed host, since the proxy has no concept of which bytes are a secret. Closing that requires the proxy to hold the secret itself and inject it into outgoing requests (Authorization header, DB connection string, etc.) so the child process's env never contains the raw value — a real architectural change (stateful, protocol-aware per credential type: HTTP header vs. Postgres wire vs. CLI arg), not an incremental patch on the current proxy. Deliberately not built speculatively; scope it against a concrete credential case (e.g. "Postgres URL via env, must not be visible to the child process") when one shows up, rather than guessing the shape now.

3. **Namespaces**:
   - Uses `CLONE_NEWNS` to isolate mounts. 
   - Mounts `/dev/null` over `.env` files dynamically during startup to prevent token leakage.

4. **Workspace Integrity Tripwire**:
   - `sbox lock`: Computes a stable SHA-256 content hash of all files inside `node_modules` and `.venv`.
   - The lockfile is saved in `~/.local/share/sbox/locks/<hash>.json`.
   - `sbox run`: Reads the lockfile and verifies the hash before booting the process.
   - Crucially, `~/.local` is mounted strictly Read-Only in the Landlock ruleset, making the lockfile mathematically impossible to tamper with from inside the sandbox.

## Command Execution Flow

1. **CLI Parsing** (`src/cli.rs`): Parses `run`, `lock`, or `shim` commands.
2. **Pre-flight Checks** (`src/lock.rs`): If running a command, `verify_project()` hashes the dependencies and compares them to the immutable lockfile.
3. **Policy Resolution** (`src/policy.rs`): Translates CLI flags (`--allow-net-out`, `--allow-env`) into a `CommandPolicy` defining paths and capabilities.
4. **Environment Scrubbing** (`src/main.rs`): Purges dangerous tokens.
5. **Sandbox Initialization** (`src/sandbox.rs`):
   - Generates Landlock rules based on the `CommandPolicy`.
   - Unshares mount namespaces.
   - Masks `.env` files.
6. **Execution**: Spawns the subprocess directly inside the locked environment.

## Design Rules

1. **Do not use external dependencies for isolation**: Stick to kernel features (Landlock, namespaces, seccomp) plus plain `std` where the kernel primitive is provably too coarse (see egress proxy above). No containers, no new crates for isolation logic.
2. **Fail closed**: Security policies must deny access by default.
3. **Keep it fast**: The entire boot sequence must execute in under 2ms. Avoid heavy I/O outside of explicit `lock` operations.
4. **No UI clutter**: Minimal logging. Output should primarily be the subprocess stdout/stderr unless an error occurs in the sandbox itself.
