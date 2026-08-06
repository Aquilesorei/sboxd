# sbox Architecture

`sbox` is designed with a minimal, zero-config architecture to keep code maintainable, readable, and lightning-fast.

---

## Code Base Structure

```
src/
├── main.rs       # Entrypoint & CLI dispatch (~100 lines)
├── cli.rs        # Clap CLI definitions: run/lock/shim (~80 lines)
├── policy.rs     # CommandPolicy resolution (RO/RW paths, net/env flags) (~80 lines)
├── sandbox.rs    # Landlock LSM, namespaces, .env masking, egress-proxy wiring (~235 lines)
├── proxy.rs      # Host-allowlisted local HTTP/HTTPS egress proxy (~145 lines)
├── lock.rs       # Workspace dependency-hash tripwire (sbox lock / verify) (~170 lines)
├── shim.rs       # Transparent shell shim generator & verifier (~70 lines)
├── platform.rs   # OS & path helpers
└── error.rs      # Typed error definitions (thiserror)
```

---

## Architectural Principles

1. **Zero Config First**: No `sbox.yaml` required. Sensible secure defaults, opt-in flags for anything riskier.
2. **Native Host Binary Execution**: Executes the developer's installed tools (`npm`, `cargo`, `uv`, `python`) directly — no container engine, no image layer.
3. **No Heavy Design Patterns**: Standard Rust idioms (`CommandExt`, `Landlock`, `clap`, plain `std::net`) — no trait factories, no plugin system, no config DSL.
4. **Instant Startup**: Boot sequence targets < 2ms. The one exception is `sbox lock`, which does deliberate, explicit I/O (hashing `node_modules`/`.venv`/`vendor`) — that cost is paid once, at lock time, not on every `run`.

## Module Responsibilities

- **`policy.rs`**: turns CLI flags into a `CommandPolicy` — resolves the binary via `PATH`, decides read-write paths (cwd, `/tmp`, `/var/tmp`, `~/.cache`, dependency dirs), and carries `network_enabled` / `allow_env` / `allow_net_out` through unmodified for `sandbox.rs` to enforce.
- **`sandbox.rs`**: the enforcement point. Builds the Landlock ruleset (filesystem + network), decides whether to unshare `CLONE_NEWNET`, spawns the egress proxy when host-restricted networking is requested, masks `.env` files via bind mount, scrubs sensitive env vars, then `pre_exec`s the child into the restricted process.
- **`proxy.rs`**: standalone, no dependency on Landlock or the rest of the sandbox — a plain host-allowlisting HTTP/HTTPS forward proxy. Kept separate because it's the one piece doing non-trivial I/O logic (parsing request heads, splicing streams) and is unit-testable in isolation.
- **`lock.rs`**: the integrity tripwire. Pure hashing + JSON, no kernel interaction — the tamper-resistance comes entirely from *where* the lockfile lives (`~/.local/share/sbox/locks/`, mounted read-only by Landlock during `run`), not from anything clever in this module.
- **`shim.rs`**: writes tiny `exec sbox run -- <tool> "$@"` wrapper scripts so `npm`, `cargo`, `uv`, etc. route through sbox transparently once the shim dir is on `PATH`.
