# sbox Security Model

## Threat Model

`sbox` protects against **malicious postinstall scripts and supply chain attacks** during package installation and build processes (`npm install`, `cargo build`, `pip install`, `uv sync`), and during ordinary runs of untrusted or third-party code.

### Attacks Contained by sbox

| Threat Vector | Unprotected Execution | Protected by sbox |
|---|---|---|
| **Reading SSH / AWS Keys** | Attacker reads `~/.ssh/id_rsa` or `~/.aws/credentials` | **Blocked**: not covered by any Landlock rule, so inaccessible by default |
| **Reading `.env` Secrets** | Attacker reads `DATABASE_URL`, `SECRET_KEY` etc. from `.env` on disk | **Blocked**: `.env`/`.env.*` bind-mounted to `/dev/null` unless `--allow-env` |
| **Stealing Environment Secrets** | Attacker dumps `AWS_SECRET_ACCESS_KEY` or `NPM_TOKEN` from the process env | **Blocked**: env scrubbing removes token/secret/key-shaped vars before spawn, unless `--allow-env` |
| **Exfiltrating Data to Arbitrary Hosts** | Attacker sends stolen data to any remote C2 server | **Blocked by default** (`--offline`: `CLONE_NEWNET`, no route at all) or **host-restricted** (`--allow-net-out=<hosts>`: only listed hosts reachable, everything else 403s at the local egress proxy) |
| **Modifying Host Executables / Toolchain** | Attacker writes malicious scripts to `/usr/bin`, `~/.cargo/bin` | **Blocked**: system dirs and toolchain dirs mounted Read-Only |
| **Tampering with Dependencies for a Future Run** | Attacker patches a file inside `node_modules`/`.venv` so the *next* run executes malicious code | **Blocked**: `sbox lock` hash mismatch refuses to boot; the lockfile itself lives under `~/.local`, which is Landlock Read-Only, so nothing inside the sandbox can rewrite it |

---

## Security Guarantees

- **Unprivileged Execution**: no `sudo`, no root, no setuid binary. Landlock and unprivileged user+mount namespaces are available to any process on Linux 5.13+.
- **Kernel-Enforced, Not Convention-Enforced**: filesystem access, network access, and the lockfile's immutability are all enforced by the kernel (Landlock LSM, mount namespaces), not by application-level checks a malicious child could bypass.
- **Fail Closed**: no explicit rule means no access — this applies to both filesystem paths and (with `--allow-net-out`) network hosts.
- **Environment Scrubbing & `.env` Masking**: two independent mechanisms (one for the process env, one for files on disk) so a secret has to be missed by both to leak, by default.

---

## Known Limitations

These are documented gaps, not oversights — see `CLAUDE.md` for the reasoning behind each and what it would take to close them.

- **Egress proxy is host-scoped, not secret-scoped.** Run a command with both `--allow-env` and `--allow-net-out=<allowed-host>`, and any code in that process — including a malicious nested dependency — can still send `.env` secrets to that *same allowed host*. sbox stops exfiltration to arbitrary/unlisted hosts; it does not stop misuse of a host you've already told it to trust. Closing that gap means the proxy holding the secret itself and injecting it into outgoing requests, so the raw value never reaches the child's environment — a real architecture change, not a flag. Not built speculatively; needs a concrete credential case to scope correctly.
- **Egress proxy's Landlock exception is port-scoped, not address-scoped.** Landlock has no notion of "this port, but only when the destination is 127.0.0.1" — the `NetPort` rule that lets the sandboxed process reach the proxy technically also permits connecting to any *external* host that happens to listen on the exact same ephemeral port number. The port is randomized per run specifically to make this collision astronomically unlikely. Treat it as a documented ceiling of the primitive, not a bug.
- **No payload/request-shape inspection.** The egress proxy allowlists by hostname (`CONNECT`/`Host:` header) only — it does not parse or restrict *what* is sent to an allowed host. A legitimate API call and a malicious one that piggybacks on the same allowlisted host are indistinguishable to sbox.
- **`--allow-net-out` with no hosts (bare flag) is unrestricted and unenforced**, kept only for backward compatibility. It bypasses the egress proxy entirely. Prefer the host-restricted form; the bare form prints a deprecation warning at run time.
