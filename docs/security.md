# Security Model

## What sbox is defending against

The threat is **malicious postinstall scripts**. When you run `npm install`, every package you install can declare a `postinstall` script that runs automatically on your machine with your user's full privileges. The same is true for `pip install` (setup.py), `cargo build` (build.rs), and similar.

Most of the time these scripts do legitimate things: compile native modules, download platform-specific binaries, set up configuration. Sometimes they do not. The March 2026 Axios supply chain attack delivered a remote-access trojan via a postinstall hook. The script ran on developer machines and CI servers, read credentials from the environment and filesystem, and exfiltrated them.

sbox's answer: run those scripts inside a container that cannot see your credentials, cannot reach the internet (or can only reach the registry), and cannot write outside a controlled area. If the script is malicious, it runs in a box with nothing worth stealing.

---

## What happens during a sandboxed install

When you run `sbox run -- npm install`:

1. sbox reads `sbox.yaml`, resolves the policy, and builds a container command
2. A rootless Podman container starts with:
   - Your project directory bind-mounted read-only at `/workspace`
   - `node_modules/` bind-mounted read-write at `/workspace/node_modules/`
   - No other filesystem access
   - Only the env vars you explicitly allowed
   - No network (or only allowed registry IPs)
3. npm runs inside the container, downloads packages, runs postinstall scripts
4. The container exits
5. `node_modules/` on your host contains the installed packages

The postinstall scripts ran in step 3. They saw an empty home directory with no SSH keys, no AWS credentials, no `.npmrc` token. They could not reach `evil.attacker.com`. They could not write outside `node_modules/`. They could not escalate privileges.

---

## What each control does

### Credential masking

Files listed in `exclude_paths` are replaced with empty `/dev/null` bind mounts inside the container. The postinstall script sees a zero-byte file instead of the real content.

```yaml
workspace:
  exclude_paths:
    - .npmrc          # masks ./npmrc in the workspace
    - ".ssh/*"        # masks all files inside .ssh/
    - ".aws/*"        # masks all files inside .aws/
```

The home directory is never mounted. A postinstall script that tries `fs.readFileSync(os.homedir() + '/.ssh/id_rsa')` hits the container's own home directory, which has nothing in it — not your key.

Files in the workspace that match `exclude_paths` are masked. For example, if your project has a `.env` file with API keys, it appears empty inside the container.

### Environment filtering

Every environment variable on your host is stripped before the container starts. Only variables you explicitly list in `pass_through` are forwarded. Variables in `deny` are blocked even if they appear in `pass_through` or `set`.

```yaml
environment:
  pass_through:
    - TERM
    - CI
  deny:
    - NPM_TOKEN
    - AWS_SECRET_ACCESS_KEY
    - GITHUB_TOKEN
```

A postinstall script that reads `process.env.NPM_TOKEN` gets `undefined`. The variable was never in the container's environment.

`deny` always wins. If a variable is in both `set` and `deny`, it is not passed to the container.

### Network isolation

With `network: off`, the container has no network interface. DNS fails. TCP connections fail immediately. There is nothing to reach.

With `network_allow`, the container's DNS is pointed at a non-routable address so arbitrary lookups fail, while specific registry IPs are injected directly into `/etc/hosts`. The container can reach the registry but not arbitrary internet hosts.

See [network.md](network.md) for the full explanation of how this works and where the gaps are.

### Read-only workspace

The workspace is mounted read-only by default. A postinstall script cannot modify your source files, your git history, your CI config, or your shell profile. Path traversal attempts (`../../etc/crontab`) fail with `EACCES`.

Only `writable_paths` are writable:

```yaml
workspace:
  writable: false
  writable_paths:
    - node_modules    # output directory
```

### No new privileges

```yaml
profiles:
  install:
    no_new_privileges: true
```

This sets `--security-opt no-new-privileges` on the container. The process inside cannot gain additional Linux capabilities via setuid binaries. Combined with rootless Podman, the container process is your UID on the host — never root.

---

## What sbox does NOT protect against

### Post-install artifacts running on the host

After `sbox run -- npm install` exits, `node_modules/` is on your host. If you then run `npm run build` on the host (outside sbox), node executes code from `node_modules/` with your full host privileges.

A malicious package can plant code in `node_modules/.bin/` during install. That code does nothing during the sandboxed install. It executes when you run your build script outside the sandbox.

**The fix:** route build and run commands through sbox too. See [Progressive adoption](adoption.md), Stage 3.

### Hardcoded IP addresses with `network_allow`

`network_allow` enforces by DNS. It cannot stop a postinstall script that skips DNS and connects directly to a hardcoded IP. Cloud metadata endpoints (`169.254.169.254`) are reachable this way.

**The fix:** use `network: off` with a pre-populated cache, or the two-phase install pattern. See [network.md](network.md).

### Resource exhaustion

sbox does not limit CPU or memory by default. A malicious package can spin up threads and consume resources inside the container. This affects performance but not host security.

### Container escape

sbox relies on Linux namespaces for isolation. A kernel vulnerability that allows container escape would bypass all sbox controls. Rootless Podman reduces the attack surface (no root daemon, no setuid binaries in the container runtime path), but is not a guarantee.

---

## Adversarial test results

The `tests/adversarial/` directory contains a test harness that runs a real malicious npm package inside a sandbox and verifies each attack pattern was blocked.

```
── Credential reads ─────────────────────────────────────────────────
  ✓ PASS  read ~/.ssh/id_ed25519      [BLOCKED]
  ✓ PASS  read ~/.ssh/id_rsa         [BLOCKED]
  ✓ PASS  read ~/.npmrc               [BLOCKED]
  ✓ PASS  read ~/.netrc               [BLOCKED]
  ✓ PASS  read ~/.aws/credentials     [BLOCKED]
  ✓ PASS  read ~/.docker/config.json  [BLOCKED]
  ✓ PASS  read ~/.kube/config         [BLOCKED]

── Environment leaks ────────────────────────────────────────────────
  ✓ PASS  dump sensitive env vars     [BLOCKED]

── Network exfiltration ─────────────────────────────────────────────
  ✓ PASS  HTTP exfil to attacker      [BLOCKED]
  ✓ PASS  raw TCP to 1.1.1.1:443      [BLOCKED]
  ✓ PASS  curl to external URL        [BLOCKED]
  ✓ PASS  wget to external URL        [BLOCKED]

── Workspace writes ──────────────────────────────────────────────────
  ✓ PASS  write via path traversal    [BLOCKED]
  ✓ PASS  write .git/hooks/pre-commit [BLOCKED]

── Privilege escalation ─────────────────────────────────────────────
  ✓ PASS  sudo id                     [BLOCKED]
  ✓ PASS  check if running as root    [BLOCKED]
  ✓ PASS  read /etc/shadow            [BLOCKED]

Results: 17 passed, 0 failed, 0 skipped
```

See [adversarial-testing.md](adversarial-testing.md) for how to run this yourself.

---

## Strict mode

```bash
sbox --strict-security run -- npm install
```

or in config:

```yaml
runtime:
  strict_security: true
```

Strict mode adds hard requirements on top of the normal policy:

- **Image must be pinned.** If `image.digest` is not set, execution is refused. This prevents silent image drift — you know exactly which image you reviewed.
- **Lockfile must exist** for install-style commands. `npm install` without a `package-lock.json` installs latest versions, which may have changed since you last audited. Strict mode refuses until the lockfile is present.
- **No sensitive pass-through.** If any variable in `pass_through` looks like a credential (contains "token", "secret", "key", "password", "credential"), strict mode refuses.

Use strict mode in CI where you want hard guarantees, not just best-effort defaults.
