# How sbox Works

`sbox` is a zero-config, native security runner for Linux that executes development commands (`npm`, `cargo`, `uv`, `python`, `go`, ...) inside an unprivileged kernel sandbox — no Docker, no Podman, no daemon.

---

## 1. Process Execution Flow

Three ways to invoke it, all converging on the same path:

```
sbox npm install              # bare form (external subcommand)
sbox run npm install          # explicit form
sbox run --allow-env --allow-net-out=registry.npmjs.org npm install
```

```
User Invocation
     │
     ├── 1. Lock Verification (src/lock.rs):
     │      - If node_modules/.venv/vendor exists, its hash must match
     │        the stored lockfile, or the run is refused outright.
     ├── 2. Binary Lookup: PATH search resolves the real binary (e.g. `npm`)
     ├── 3. Policy Resolution (src/policy.rs):
     │      - network_enabled = !offline (ON unless -n/--offline)
     │      - allow_env, allow_net_out passed through as-is
     │      - Read-write paths computed: cwd, /tmp, /var/tmp, ~/.cache,
     │        + any existing node_modules/target/.venv/vendor/dist/build
     ├── 4. Egress Proxy (src/proxy.rs), only if --allow-net-out=<hosts>:
     │      - Local HTTP/HTTPS proxy spawned on an ephemeral 127.0.0.1 port
     │      - Child's HTTPS_PROXY/HTTP_PROXY env vars point at it
     ├── 5. Landlock Ruleset Assembly (src/sandbox.rs):
     │      - Read-Only: /usr, /lib, /lib64, /etc, /bin, /sbin, /dev, /proc,
     │        toolchain dirs (~/.cargo, ~/.rustup, ~/.nvm, ~/.pyenv, ~/.bun,
     │        ~/.local), every directory on $PATH
     │      - Read-Write: the paths from step 3
     │      - Network: ConnectTcp denied by default; if the egress proxy is
     │        running, exactly its one port is exempted (NetPort rule) —
     │        everything else outbound still fails closed
     │      - Everything not explicitly ruled (~/.ssh, ~/.aws, ...) is
     │        unreachable by omission, not by a special-case block
     ├── 6. Env Scrubbing (src/sandbox.rs): strips AWS_*, GITHUB_TOKEN,
     │      NPM_TOKEN, SLACK_TOKEN, SECRET_*, PRIVATE_KEY, DATABASE_URL,
     │      PASSWORD, and anything containing SECRET/TOKEN/KEY — unless
     │      --allow-env is set
     ├── 7. Child Spawn via pre_exec Hook:
     │      - libc::unshare(CLONE_NEWUSER | CLONE_NEWNS [| CLONE_NEWNET if offline])
     │      - bind-mount /dev/null over .env / .env.* in cwd (unless --allow-env)
     │      - ruleset.restrict_self()
     └── 8. execve(real binary, args)
```

---

## 2. Kernel Isolation Technologies

### Landlock LSM (Filesystem + Network, ABI V4)
Landlock is a Linux Security Module (Linux 5.13+) letting a process restrict its *own* permissions — no root needed. It's default-deny: any path or network access not explicitly ruled is inaccessible once `restrict_self()` runs.
- **Read-Only System & Toolchains**: `/usr`, `/lib`, `/lib64`, `/etc`, `/bin`, `/sbin`, `/dev`, `/proc`, plus toolchain dirs and everything on `$PATH`.
- **Read-Write Workspace**: cwd, `/tmp`, `/var/tmp`, `~/.cache`, and any dependency/build dirs that already exist.
- **Inaccessible by omission**: `~/.ssh`, `~/.aws`, and anything else outside the rules above — no rule means no access, there's nothing to "unblock."
- **Network**: `AccessNet::ConnectTcp` is denied by default (`BindTcp` is left unhandled, so local dev servers can still bind a port). With `--allow-net-out=<hosts>`, one `NetPort` rule opens exactly the local egress proxy's port — see below.

### Egress Proxy (`src/proxy.rs`)
Landlock's network rules key on **port number, not destination host** — there's no way to tell Landlock "allow api.stripe.com, block everything else" directly. So `sbox` opens exactly one port (a local proxy it spawns) and puts the host allowlist enforcement *there* instead:
- `--allow-net-out=host1,host2` → proxy accepts `CONNECT` (HTTPS tunnels) and plain HTTP, checks the target host against the allowlist, and only then opens the real upstream connection. Off-list hosts get `403` and are logged to stderr.
- `--allow-net-out` with no hosts still works for backward compatibility — no proxy, no Landlock port restriction, unrestricted outbound (prints a deprecation warning).
- **Known ceiling**: the Landlock exception is scoped to the proxy's *port number*, not its address — a remote host that happened to listen on the same ephemeral port would technically also be directly reachable. The port is randomized per run to make that collision astronomically unlikely, not to make it impossible. See `docs/security.md`.

### Namespaces (`CLONE_NEWNS`, `CLONE_NEWUSER`, `CLONE_NEWNET`)
- `CLONE_NEWUSER | CLONE_NEWNS` are unshared on every run — this is what allows the bind-mount below without root.
- `CLONE_NEWNET` is added only for `--offline`/`-n`: outbound sockets fail immediately (`ENETUNREACH`), stronger and cheaper than the Landlock-level deny for builds/tests that need zero network.
- **`.env` masking**: before exec, `sbox` bind-mounts `/dev/null` over every `.env` / `.env.*` file in the cwd, unless `--allow-env` is passed. The process sees the file exists but reads nothing from it.

### Environment Variable Scrubbing
Independent of the `.env` mask above — this strips secrets that are already *in the process environment* (not just in a file) before the child is spawned: `AWS_*`, `GITHUB_TOKEN`, `NPM_TOKEN`, `SLACK_TOKEN`, `SECRET_*`, `PRIVATE_KEY`, `DATABASE_URL`, `PASSWORD`, plus any key containing `SECRET`, `TOKEN`, or `KEY`. Skipped entirely with `--allow-env`.

### Workspace Integrity Tripwire (`src/lock.rs`)
Separate from process isolation — this defends the *next* run against tampering during *this* one:
- `sbox lock` hashes every file under `node_modules`/`.venv`/`vendor` (SHA-256, sorted paths, deterministic) and writes it to `~/.local/share/sbox/locks/<workspace-path-hash>.json`.
- `sbox run` recomputes the hash and refuses to launch on mismatch — including refusing to launch at all if dependencies exist but were never locked.
- The lockfile survives a compromised `run` because `~/.local` is mounted **read-only** by the same Landlock ruleset — nothing inside the sandbox can rewrite it, mathematically, not just by convention.
