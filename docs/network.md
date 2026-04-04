# Network Security

> **The key risk:** enabling network for package downloads also enables postinstall scripts to reach the internet. There is no perfect solution. The options below explain the trade-offs clearly so you can make an informed choice.

---

## Why this is hard

`npm install` does two things inside the same process:

1. **Download** — fetches tarballs from the registry. Needs the internet.
2. **Postinstall** — runs arbitrary scripts from the downloaded packages. Should not have the internet.

If you run the whole install with `network: off`, npm cannot download anything. If you use `network: on`, postinstall scripts can send your environment variables to an attacker's server. Both options happen inside the same container.

The Axios supply chain attack (March 2026) is exactly this scenario: a malicious postinstall hook used the network to exfiltrate credentials from the developer's environment. A sandboxed install with `network: off` would have made that exfiltration impossible — but only if the packages were already available locally.

---

## Option 1 — `network: off` with local packages

**Strongest isolation. No exfiltration possible.**

The container has no network stack at all. DNS fails. TCP/UDP connections fail immediately with `ENETUNREACH`. There is no loopback interface.

```yaml
profiles:
  install:
    mode: sandbox
    network: off
```

This works when packages are already local: a vendored `node_modules/`, a pre-fetched cache volume, a `.tgz` tarball, or a private registry running on localhost.

In CI pipelines that pre-populate a cache layer before the sandboxed install step, this is the recommended approach. First warm the cache (on the host or in a separate step with network on), then run the install with `network: off` against the cache.

**When to use:** CI with a warm cache, vendored dependencies, local development after the first install.

---

## Option 2 — `network_allow` with a registry allowlist

**Practical for most teams. Blocks common exfiltration patterns.**

Allow only specific registries. Postinstall scripts cannot reach arbitrary hosts.

```yaml
profiles:
  install:
    mode: sandbox
    network: on
    network_allow:
      - "*.npmjs.org"
```

**How it works under the hood:**

sbox points the container's DNS resolver at a non-routable address (`192.0.2.1`). Any hostname lookup that is not on the allowlist times out — the container never gets an IP for it.

For hostnames on the allowlist, sbox resolves them on the host at plan time and injects the IPs directly into the container's `/etc/hosts` via `--add-host`. The container can reach `registry.npmjs.org` because its IP is already there, bypassing DNS entirely. `evil.attacker.com` never gets resolved.

You can see what was resolved by running `sbox plan`:

```
network_allow: [resolved] registry.npmjs.org=104.x.x.x, npmjs.org=104.x.x.x; [patterns] *.npmjs.org
```

**The gap:** a postinstall script that hardcodes an IP address bypasses DNS entirely. If an attacker controls a script and knows a reachable IP — including a cloud metadata endpoint like `169.254.169.254` — `network_allow` does not stop it. For that level of protection, you need `network: off` or a kernel-level firewall.

**When to use:** most teams doing live installs from a public registry. Covers the vast majority of real postinstall exfiltration attempts, which use domain names, not hardcoded IPs.

### Allowlists by ecosystem

**npm / pnpm / yarn:**
```yaml
network_allow:
  - "*.npmjs.org"
  - "*.yarnpkg.com"
```

**Python (pip / uv / poetry):**
```yaml
network_allow:
  - "*.pypi.org"
  - files.pythonhosted.org
```

**Rust (cargo):**
```yaml
network_allow:
  - "*.crates.io"
  - static.crates.io
```

**Go modules:**
```yaml
network_allow:
  - proxy.golang.org
  - sum.golang.org
```

### Pattern expansion

For well-known package registry domains, sbox expands wildcard patterns to the full set of known subdomains before resolving. `*.npmjs.org` resolves `registry.npmjs.org`, `npmjs.org`, and `www.npmjs.org` — not just the bare `npmjs.org`. This prevents the case where a CDN serves the registry content from a subdomain that a naive single-host allowlist would miss.

Built-in expansion tables cover npm, yarn, PyPI, crates.io, Go, RubyGems, Maven, GitHub, and the major OCI registries.

---

## Option 3 — Two-phase install (strongest with live downloads)

**The right answer if you need both live downloads and strong postinstall isolation.**

Run the download and postinstall execution as separate steps with different network policies.

**Phase 1 — download without scripts** (network allowed, scripts disabled):

```bash
npm install --ignore-scripts
```

npm fetches all packages but skips every postinstall, prepare, and build script. No untrusted code runs.

**Phase 2 — run scripts without network** (network off):

```bash
npm rebuild
```

npm runs the install scripts against the already-downloaded packages. The network is off, so scripts cannot exfiltrate anything.

Model this in sbox with two profiles:

```yaml
profiles:
  download:
    mode: sandbox
    network: on
    network_allow:
      - "*.npmjs.org"
    writable: true
    no_new_privileges: true

  scripts:
    mode: sandbox
    network: off
    writable: true
    no_new_privileges: true

dispatch:
  npm-download:
    match:
      - "npm install --ignore-scripts*"
    profile: download

  npm-rebuild:
    match:
      - "npm rebuild*"
    profile: scripts
```

Usage:

```bash
sbox run -- npm install --ignore-scripts   # downloads, no postinstall
sbox run -- npm rebuild                    # runs postinstall, no network
```

**The gap:** some packages use `prepare` scripts that genuinely need to compile native code or run build steps during install. Those may fail without network if they try to download build tools. In practice, most packages work fine with `--ignore-scripts` + `npm rebuild`.

---

## Comparison

| | `network: off` | `network_allow` | Two-phase |
|--|----------------|-----------------|-----------|
| Live downloads | No | Yes | Yes |
| Blocks domain-based exfil | Yes | Yes | Yes (phase 2) |
| Blocks hardcoded-IP exfil | Yes | No | Yes (phase 2) |
| Complexity | Low | Low | Medium |
| Works without cache | No | Yes | Yes |

For most teams: **start with `network_allow`** pointing at your registry. If you need stronger guarantees, move to two-phase. If you have a cache, use `network: off`.
