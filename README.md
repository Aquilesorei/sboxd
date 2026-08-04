# sbox

Zero-config native Linux security sandbox for development commands.

sbox protects the host machine from supply chain attacks and malicious post-install scripts during package manager executions (`npm install`, `cargo build`, `uv sync`) without requiring Docker, Podman, or configuration files.

## Features

- **Kernel Landlock Filesystem Isolation**: Restricts filesystem access using Landlock LSM (ABI V4). System paths (`/usr`, `/lib`, `/etc`) and toolchain paths are granted read-only access. The project workspace and `/tmp` are granted read-write access. Sensitive paths (`~/.ssh`, `~/.aws`) are strictly blocked. 
- **Kernel Landlock Network Egress Blocking**: Implements a zero-trust default for outgoing TCP connections (`AccessNet::ConnectTcp`) to prevent data exfiltration. `BindTcp` is permitted to seamlessly support local development servers.
- **Namespaces & Bind Mounts**: Utilizes `CLONE_NEWNS` to bind-mount `/dev/null` over `.env` files, preventing secrets from being read during sandbox execution.
- **Workspace Hashing Tripwire**: Includes `sbox lock` to compute a stable SHA-256 content hash of dependency directories (`node_modules`, `.venv`). Lockfiles are securely stored in `~/.local/share/sbox/locks/`, which is enforced as read-only within the sandbox. The `sbox run` command verifies this hash before booting to prevent runtime dependency tampering.
- **Environment Scrubbing**: Automatically strips AWS keys, GitHub tokens, and secret environment variables prior to process execution.

## Usage

### Direct Command Execution
Run any command cleanly through sbox in any directory:

```bash
# General sandboxed execution (Network egress denied by default)
sbox run npm start

# Allow network egress
sbox run --allow-net-out npm install

# Allow environment variables to bypass scrubbing
sbox run --allow-env my_script.sh
```

### Workspace Hashing Tripwire
Snapshot the current project dependencies to prevent runtime tampering:

```bash
# Run after a trusted install to compute the content hash and lock it
sbox lock

# Run command (verifies against the lockfile before boot)
sbox run npm start
```

### Transparent Shell Shims
Intercept dev tools transparently:

```bash
sbox shim install
export PATH="$HOME/.local/share/sbox/shims:$PATH"
sbox shim verify
```

## Architecture Notes
- Execution relies entirely on Linux kernel primitives. 
- No container runtime (Docker/Podman) is required.
- Execution startup time is under 2 milliseconds.
