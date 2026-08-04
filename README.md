# sbox

> **Zero-config, instant (< 2ms) native Linux security sandbox for development commands.**

`sbox` protects your host machine from **malicious postinstall scripts and supply chain attacks** during `npm install`, `cargo build`, `uv sync`, `pip install`, and other package manager executions—without Docker, Podman, or configuration files.

---

## ⚡ Why sbox?

| Feature | Raw Command (`npm install`) | Heavy Containers (Docker/Podman) | **`sbox` (Linux Native)** |
|---|---|---|---|
| **Security** | ❌ Full Host Access | ✅ Isolated Container | ✅ **Kernel Landlock + NetNS** |
| **Setup Needed** | None | ❌ Install Docker/Podman, pull OCI images | ✅ **Zero Setup (Single Binary)** |
| **Configuration** | None | ❌ Requires 50-line YAML configs | ✅ **Zero Config Needed** |
| **Tool Version** | Host version | Container image version | ✅ **Host Machine Version** |
| **Startup Speed** | < 1ms | ❌ ~300ms – 2,000ms | ⚡ **< 2 milliseconds** |

---

## 🛡️ Threat Model: Supply Chain Attacks

When you clone an open-source project and run `npm install` or `cargo build`, lifecycle scripts (`postinstall`, `build.rs`, `setup.py`) execute arbitrary code on your computer. Malicious packages attempt to:

- Read `~/.ssh/id_rsa`, `~/.aws/credentials`, or `.env` files.
- Exfiltrate tokens (`AWS_SECRET_ACCESS_KEY`, `GITHUB_TOKEN`, `NPM_TOKEN`) over remote network sockets.
- Modify system binaries or shell RC files (`~/.bashrc`).

`sbox` runs these commands inside a **Linux Kernel Sandbox** that blocks secret theft, locks down filesystem access, and unshares network access.

---

## 🚀 Quick Start

### 1. Direct Command Execution
Run any command cleanly through `sbox` in any directory:

```bash
# Sandboxed package installation (Network: ON, Filesystem: Restricted, Env: Scrubbed)
sbox npm install
sbox uv sync
sbox cargo add serde

# Sandboxed builds & tests (Network: OFF, Filesystem: Restricted, Env: Scrubbed)
sbox cargo build --release
sbox npm test
sbox python main.py
```

### 2. Transparent Shell Shims
Intercept your dev tools transparently so you never forget to sandbox:

```bash
# Install shims into ~/.local/share/sbox/shims
sbox shim install

# Add shims to your ~/.bashrc or ~/.zshrc:
export PATH="$HOME/.local/share/sbox/shims:$PATH"

# Verify active shims
sbox shim verify
```

---

## 🔬 How It Works (Kernel Primitives)

`sbox` uses unprivileged Linux kernel security features:

1. **Landlock LSM**: Restricts filesystem access. Grants Read-Only rights to system paths (`/usr`, `/lib`, `/etc`) and toolchain paths (`~/.cargo`, `~/.rustup`, `~/.nvm`, etc.), Read/Write to the project workspace and `/tmp`, while hard-blocking access to sensitive files (`~/.ssh`, `~/.aws`, `.env`).
2. **Network Namespaces (`CLONE_NEWNET`)**: Unshares outbound network access for builds and scripts so malicious postinstall payloads cannot exfiltrate data over HTTP/DNS.
3. **Environment Scrubbing**: Automatically strips AWS keys, GitHub tokens, and secret environment variables prior to process execution.

For detailed architecture docs, see [`docs/how-it-works.md`](file:///home/aquiles/RustroverProjects/sbox/docs/how-it-works.md) and [`docs/security.md`](file:///home/aquiles/RustroverProjects/sbox/docs/security.md).

---

## 📜 License

MIT License. Created by Aquiles.
