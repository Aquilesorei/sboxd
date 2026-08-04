# sbox Security Model

## Threat Model

`sbox` protects against **malicious postinstall scripts and supply chain attacks** during package installation and build processes (`npm install`, `cargo build`, `pip install`, `uv sync`).

### Attacks Contained by sbox

| Threat Vector | Unprotected Execution | Protected by sbox |
|---|---|---|
| **Reading SSH / AWS Keys** | Attacker reads `~/.ssh/id_rsa` or `~/.aws/credentials` | **Blocked**: Landlock denies access to `~/.ssh` and `~/.aws` |
| **Stealing Environment Secrets** | Attacker dumps `AWS_SECRET_ACCESS_KEY` or `NPM_TOKEN` | **Blocked**: Environment scrubbing removes sensitive keys |
| **Exfiltrating Data Over Network** | Attacker sends stolen code to remote C2 server | **Blocked**: `unshare(CLONE_NEWNET)` disables outbound network |
| **Modifying Host Executables** | Attacker writes malicious scripts to `/usr/bin` | **Blocked**: `/usr` and `/bin` are mounted Read-Only |

---

## Security Guarantees

- **Unprivileged Execution**: Does not require `sudo` or root privileges.
- **Kernel-Enforced Limits**: Landlock and Network Namespaces are enforced by the Linux kernel.
- **Environment Scrubbing**: Automatically strips token patterns before child process spawn.
