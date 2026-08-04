# How sbox Works

`sbox` is a zero-config, native security runner for Linux that executes development commands (`npm`, `cargo`, `uv`, `python`, `go`) inside an unprivileged kernel sandbox.

---

## 1. Process Execution Flow

When you run `sbox <command> [args...]`:

```
User Invocation: $ sbox cargo build
     │
     ├── 1. Binary Lookup: PATH search resolves `/home/user/.cargo/bin/cargo`
     ├── 2. Policy Resolution: 
     │      - Detects command intent (`build` -> Network OFF, FS Restricted)
     ├── 3. Landlock Ruleset Assembly:
     │      - Read-Only: /usr, /lib, /etc, ~/.cargo, ~/.rustup
     │      - Read-Write: CWD, /tmp, /var/tmp
     │      - Blocked: ~/.ssh, ~/.aws, .env
     ├── 4. Child Spawn via pre_exec Hook:
     │      - libc::unshare(CLONE_NEWUSER | CLONE_NEWNET)
     │      - ruleset.restrict_self()
     └── 5. execve(cargo, args) (< 2ms startup time)
```

---

## 2. Kernel Isolation Technologies

### Landlock LSM (Filesystem)
Landlock is a Linux Security Module (Linux 5.13+) allowing processes to restrict their own filesystem permissions.
- **Read-Only System & Toolchains**: `/usr`, `/lib`, `/etc`, and user toolchain directories (`~/.cargo`, `~/.rustup`, `~/.nvm`, `~/.pyenv`, `~/.bun`).
- **Read-Write Workspace**: CWD, `/tmp`, `/var/tmp`, and target build directories (`node_modules`, `target`, `.venv`).
- **Access Denied**: Sensitive credential paths (`~/.ssh`, `~/.aws`, `.env`) return `Permission Denied` (EACCES).

### Network Namespaces (`CLONE_NEWNET`)
`sbox` calls `unshare(CLONE_NEWUSER | CLONE_NEWNET)` before launching untrusted processes:
- **Builds & Scripts**: Outbound socket connections fail immediately with `ENETUNREACH`.
- **Package Installs**: Network access is enabled while preserving Landlock filesystem restrictions.

### Environment Variable Scrubbing
`sbox` strips sensitive keys from the environment before process execution:
- Removes: `AWS_*`, `GITHUB_TOKEN`, `NPM_TOKEN`, `SLACK_TOKEN`, `SECRET_*`, `PRIVATE_KEY_*`, `DATABASE_URL`.
