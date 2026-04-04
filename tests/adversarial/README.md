# Adversarial Test Suite

Tests that verify sbox actually contains common postinstall attack patterns.

## Requirements

- A **VM or disposable machine** — run these tests on a host you can snapshot/revert.
  If a test reveals a containment bypass, the host may be compromised.
- Rootless Podman installed and working (`podman info | grep rootless`)
- `sbox` binary in PATH

## Usage

```bash
# Run all checks against a sandboxed npm install
./tests/adversarial/run.sh

# Run only a specific attack category
./tests/adversarial/run.sh --only credential-read

# Run with a custom malicious package (local tgz)
./tests/adversarial/run.sh --package ./evil-pkg-1.0.0.tgz
```

## What is checked

| Category | Attack | Expected result |
|----------|--------|-----------------|
| credential-read | postinstall reads `~/.ssh/id_ed25519` | file masked, read returns empty |
| credential-read | postinstall reads `~/.npmrc` | file masked, read returns empty |
| credential-read | postinstall reads `~/.aws/credentials` | file not mounted, read fails |
| env-leak | postinstall dumps `process.env` to stdout | sensitive vars absent from output |
| env-leak | postinstall exfiltrates env via HTTP | network off, connection refused |
| network-exfil | postinstall calls `curl` to attacker server | network off, curl fails |
| network-exfil | postinstall opens raw TCP socket | network off, connection refused |
| workspace-write | postinstall writes to `../../../etc/crontab` | read-only workspace, write fails |
| workspace-write | postinstall modifies `.git/hooks/pre-commit` | read-only workspace, write fails |
| privilege-escalation | postinstall calls `sudo` | no-new-privileges, sudo fails |
| privilege-escalation | postinstall calls `su` | no-new-privileges, su fails |
