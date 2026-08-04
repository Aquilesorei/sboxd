# Changelog

All notable changes to `sbox` will be documented in this file.

## [0.2.0] - 2026-08-04

### 🚀 Architecture Pivot: Zero-Config Linux-Native Sandbox

`sbox` has been completely rewritten from a heavy container orchestration framework into a lightning-fast (< 2ms), zero-config Linux native security sandbox.

#### Added
- **Linux Kernel Sandboxing Engine (`Landlock LSM`)**: Restricts filesystem access to Read-Only for `/usr`, `/lib`, `/etc`, and home toolchain directories (`~/.cargo`, `~/.rustup`, `~/.nvm`, etc.), while restricting Read/Write access to the project workspace and `/tmp`.
- **Network Namespace Isolation (`CLONE_NEWNET`)**: Automatically unshares network namespaces for build, test, and script commands (`sbox cargo build`, `sbox npm test`), while allowing network access for package installs (`sbox npm install`).
- **Automated Environment Scrubbing**: Strips sensitive credentials (`AWS_*`, `GITHUB_TOKEN`, `NPM_TOKEN`, `SECRET_*`) before child process execution.
- **Zero-Config Command Execution**: Run `sbox <command>` in any directory—no `sbox.yaml` file required.
- **Transparent Shell Shims**: Updated `sbox shim install` and `sbox shim verify` for seamless command interception.

#### Removed
- Removed heavy Docker and Podman container backend dependencies.
- Removed complex multi-profile `sbox.yaml` configuration resolution engines.
- Removed skopeo digest verification policies and domain regex firewall layers.
