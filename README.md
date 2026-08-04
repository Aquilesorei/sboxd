# sbox

**`sbox`** is a zero-config, ultra-fast (<2ms) native security sandbox for Linux developers.

It protects your system from **malicious postinstall scripts and supply chain attacks** during `npm install`, `cargo build`, `uv sync`, `pip install`, and other package execution workflows—without needing Docker, Podman, or configuration files.

---

## 🔒 Threat Model: Supply Chain Attacks

When you clone an open-source repo and run `npm install` or `cargo build`, package lifecycle scripts (`postinstall`, `build.rs`, `setup.py`) execute arbitrary code on your machine. Malicious packages attempt to:

- Read `~/.ssh/id_rsa`, `~/.aws/credentials`, `~/.netrc`, or `.env` files.
- Exfiltrate secrets (`AWS_SECRET_ACCESS_KEY`, `GITHUB_TOKEN`, `NPM_TOKEN`) over outbound network sockets.
- Modify host binary locations or shell startup files (`~/.bashrc`).

`sbox` wraps these commands in a **Linux Kernel Sandbox** that blocks secret theft, locks down filesystem access, and unshares outbound network access.

---

## 🚀 Key Features

- **Zero Config**: John installs `sbox` and runs `sbox npm install` or `sbox cargo build` in any project directory. No `sbox.yaml` file needed.
- **Zero Latency (< 2ms)**: Uses your host machine's existing toolchain (`npm`, `cargo`, `python`, `uv`) directly—no container daemons or image pulls.
- **Landlock LSM Filesystem Lockdown**: Read-only system access (`/usr`, `/lib`, `/etc`) and CWD read/write. `~/.ssh`, `~/.aws`, `.env`, and secret files are hard-blocked.
- **Network Isolation**: Automatically unshares network namespaces (`CLONE_NEWNET`) for builds and tests, allowing network only for package install/sync steps.
- **Environment Scrubbing**: Automatically strips AWS keys, GitHub tokens, and secret environment variables before process execution.
- **Transparent Shell Shims**: Intercept `npm`, `cargo`, `uv`, etc., transparently via `sbox shim install`.

---

## 📦 Usage

### Direct Execution
```bash
# Sandboxed package installation (Network: ON, Filesystem: Locked, Env: Scrubbed)
sbox npm install
sbox uv sync
sbox cargo add serde

# Sandboxed builds & tests (Network: OFF, Filesystem: Locked, Env: Scrubbed)
sbox cargo build --release
sbox npm test
sbox python main.py
```

### Transparent Shims
```bash
# Install transparent shims
sbox shim install

# Add shims to your PATH in ~/.bashrc or ~/.zshrc:
export PATH="$HOME/.local/share/sbox/shims:$PATH"

# Verify active shims
sbox shim verify
```

---

## 🏗️ Architecture

`sbox` uses native Linux kernel security primitives:

1. **Landlock LSM**: Restricted filesystem ruleset created and applied via `restrict_self()`.
2. **Network Namespaces**: `libc::unshare(CLONE_NEWUSER | CLONE_NEWNET)` called in `pre_exec` hook.
3. **Environment Scrubbing**: Sensitive key removal prior to `execve()`.

See [`docs/zero-config-native-sandbox.md`](file:///home/aquiles/RustroverProjects/sbox/docs/zero-config-native-sandbox.md) for full architectural documentation.

---

## 📜 License

MIT License. Created by Aquiles.
