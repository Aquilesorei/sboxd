# sbox Usage Guide

## Quick Start

Run any dev command through `sbox` — bare form or explicit `run` both work:

```bash
sbox npm install
sbox run npm install        # equivalent, explicit form

sbox cargo build --release
sbox python main.py
```

By default: network is **ON** (outbound TCP still denied unless `--allow-net-out`), filesystem is restricted (cwd read-write, system/toolchain paths read-only, `~/.ssh`/`~/.aws` unreachable), `.env` files are masked, and secret-shaped env vars are stripped.

---

## Flags

All flags work both globally (`sbox -e run npm start`) and on the `run` subcommand (`sbox run -e npm start`).

| Flag | Short | Meaning |
|---|---|---|
| `--offline` | `-n` | Fully unshare the network namespace (`CLONE_NEWNET`). Outbound sockets fail immediately (`ENETUNREACH`). Strongest option for pure builds/tests that need zero network. |
| `--allow-env` | `-e` | Skip env-var scrubbing and `.env` file masking. Needed by anything that legitimately reads secrets (a server reading `DATABASE_URL`, etc). |
| `--allow-net-out=<hosts>` | `-o` | Allow outbound TCP, restricted to a comma-separated host allowlist, enforced by a local egress proxy. **Recommended form.** |
| `--allow-net-out` (bare) | `-o` | Allow outbound TCP, unrestricted. No host enforcement. Deprecated — kept for backward compatibility, prints a warning. |

```bash
# Package install: needs the real registry, nothing else
sbox run --allow-net-out=registry.npmjs.org npm install

# Local server: needs its DB secret and its one upstream API
sbox run --allow-env --allow-net-out=api.stripe.com,my-db-host.internal uvicorn app:app

# Pure build/test: no network at all
sbox run --offline cargo build --release

# Old unrestricted behavior (avoid — see docs/security.md)
sbox run --allow-net-out npm install
```

Multiple hosts, one flag: `--allow-net-out=host1.com,host2.com,host3.com`. Subdomains of a listed host are allowed automatically (`api.stripe.com` also permits `checkout.api.stripe.com`, not the reverse).

---

## Workspace Hashing Tripwire

Prevents a compromised install from silently poisoning dependencies for the *next* run:

```bash
# After a trusted install, snapshot node_modules/.venv/vendor
sbox lock

# Every subsequent `sbox run` (or bare `sbox <cmd>`) checks the hash first
sbox run npm start
# -> refuses to boot if dependencies changed since the last `sbox lock`,
#    or if dependencies exist but were never locked at all
```

Re-run `sbox lock` any time a dependency change is intentional (after `npm install`, `uv sync`, etc.) — it's the same command whether locking for the first time or re-locking after an expected change.

---

## Transparent Shell Shims

Route `npm`, `cargo`, `python`, and other common dev tools through `sbox` automatically, without typing `sbox run` every time:

```bash
# 1. Install shims into ~/.local/share/sbox/shims
sbox shim install

# 2. Put the shim dir ahead of the real tools on PATH
export PATH="$HOME/.local/share/sbox/shims:$PATH"

# 3. Verify the shim dir is actually active
sbox shim verify
```

Shimmed tools: `npm`, `npx`, `pnpm`, `yarn`, `bun`, `uv`, `pip`, `pip3`, `poetry`, `cargo`, `composer`, `bundle`, `node`, `python3`, `python`, `go`, `ruby`. Each shim is a two-line `exec sbox run -- <tool> "$@"` script — no flags baked in, so shimmed commands run with sbox's plain defaults (network ON but outbound-restricted, env scrubbed). Pass flags by invoking `sbox run` directly instead of the shim when you need `--allow-env`/`--allow-net-out`.
