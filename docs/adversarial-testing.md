# Adversarial Testing

The `tests/adversarial/` directory contains a test harness that verifies sbox actually contains real postinstall attack patterns — not just in theory, but by running a malicious npm package inside a sandbox and checking each attack was blocked.

## What is tested

The evil package (`tests/adversarial/evil-package/postinstall.js`) attempts 17 attack patterns:

| Category | Attack |
|----------|--------|
| Credential reads | `~/.ssh/id_ed25519`, `~/.ssh/id_rsa`, `~/.npmrc`, `~/.netrc`, `~/.aws/credentials`, `~/.docker/config.json`, `~/.kube/config` |
| Environment leak | Dump all sensitive env vars (`NPM_TOKEN`, `AWS_SECRET_ACCESS_KEY`, etc.) |
| Network exfiltration | HTTP request to attacker server, raw TCP socket to `1.1.1.1:443`, `curl`, `wget` |
| Workspace writes | Path traversal to `../../../sbox-pwned.txt`, modify `.git/hooks/pre-commit` |
| Privilege escalation | `sudo id`, check `uid === 0`, read `/etc/shadow` |

Each attempt is caught and logged — the script always exits cleanly. The test harness reads the results from a file written to `node_modules` and checks each one was `BLOCKED`.

## Requirements

- Rootless Podman installed and working
- `sbox` in PATH
- `node` and `npm` on the host (for packaging the evil package)
- Run on a VM or disposable machine

**If a containment check fails, assume the host is compromised.** The evil package runs real attack code. Use a VM you can snapshot and revert.

## Running the tests

```bash
# Full run
./tests/adversarial/run.sh

# Test only a specific category
./tests/adversarial/run.sh --only credential

# Test with a custom malicious package
./tests/adversarial/run.sh --package ./my-evil-package-1.0.0.tgz
```

## Expected output

```
sbox adversarial test
work dir: /tmp/sbox-adversarial-XXXX

► packaging evil-package...
► package: /tmp/sbox-adversarial-XXXX/sbox-adversarial-test-1.0.0.tgz

► running sandboxed npm install...
install exit code: 0

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

────────────────────────────────────────────────────────────────────
Results: 17 passed, 0 failed, 0 skipped

All checks passed — sandbox held.
```

## How the harness works

1. **Setup** — creates a temp workspace, copies `sbox.yaml`, generates fake credential files (`.ssh/id_ed25519`, `.aws/credentials`, etc.) to detect reads, generates a minimal `package.json`
2. **Packaging** — runs `npm pack` on the evil package to produce a `.tgz`
3. **Sandboxed install** — runs `sbox run -- npm install --no-save --no-package-lock --ignore-scripts=false /workspace/<pkg>.tgz`
4. **Results** — the postinstall script writes its results to `node_modules/.sbox-adversarial-results.json` (the only writable path)
5. **Evaluation** — the harness reads the results file and checks each entry against the expected outcome

The fake credentials are planted in the workspace and excluded via `exclude_paths` — the postinstall sees empty files or `ENOENT` instead of the real content.

## The sandbox config

`tests/adversarial/sbox.yaml` uses:
- `network: off` — no network at all
- `workspace.writable: false` with only `node_modules` writable
- `exclude_paths` covering `.ssh/*`, `.aws/*`, `.docker/*`, `.kube/*`, `.npmrc`, `.netrc`
- `environment.deny` for all sensitive env vars
- `no_new_privileges: true`

## Limitations

The adversarial suite uses `network: off` with a local package tarball. In a real install that downloads from a registry, you would use `network_allow` instead — see [network.md](network.md). The network exfiltration checks (`HTTP exfil`, `raw TCP socket`) are blocked by `network: off` in this test; with `network_allow`, they would also be blocked (any non-allowlisted host fails DNS resolution).

The suite does not test:
- Raw IP connections to attacker-controlled IPs when `network_allow` is used (DNS bypass)
- Artifacts that execute after the sandbox exits (code in `node_modules` running outside sbox)
- Resource exhaustion attacks (CPU/memory DoS inside the container)
