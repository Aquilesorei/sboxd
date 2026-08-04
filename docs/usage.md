# sbox Usage Guide

## Quick Start

Run any dev command through `sbox`:

```bash
# Sandboxed package installation (Network: ON, Filesystem: Restricted)
sbox npm install
sbox uv sync
sbox cargo add serde

# Sandboxed builds & tests (Network: OFF, Filesystem: Restricted)
sbox cargo build --release
sbox npm test
sbox python main.py
```

---

## Transparent Shell Shims

To run `sbox` automatically whenever you type `npm`, `cargo`, or `python`:

```bash
# 1. Install shims into ~/.local/share/sbox/shims
sbox shim install

# 2. Add shims to your shell configuration (~/.bashrc or ~/.zshrc)
export PATH="$HOME/.local/share/sbox/shims:$PATH"

# 3. Verify active shims
sbox shim verify
```
