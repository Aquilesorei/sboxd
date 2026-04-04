# Network Security

> **The key risk:** enabling network access for package downloads also enables postinstall scripts to exfiltrate data. `network: off` is the only complete block. `network_allow` with a registry allowlist is the practical middle ground. See [Option 2](#option-2--network_allow-with-an-explicit-allowlist) and [Option 3](#option-3--two-phase-install-strongest-isolation) for how to mitigate this.

## The core tension

Package installation needs the network. Postinstall scripts should not have the network. These two requirements conflict.

`npm install` (and equivalents in other ecosystems) does two things in sequence:

1. **Download** — fetches package tarballs from the registry. Requires network.
2. **Postinstall** — runs arbitrary scripts from the downloaded packages. Should not have network.

If you sandbox the whole install with `network: off`, you cannot download anything. If you use `network: on`, postinstall scripts can exfiltrate secrets.

**There is no perfect solution.** The options below are ranked by strength of isolation.

## Option 1 — `network: off` with local packages or pre-fetched caches

Works when packages are already available locally: a vendored tarball, a pre-fetched cache, or a private registry on localhost.

```yaml
profiles:
  install:
    mode: sandbox
    network: off
```

The adversarial test suite uses this approach — the evil package is a local `.tgz` passed directly to npm. In CI pipelines that pre-populate a cache volume, `network: off` is the safest option.

## Option 2 — `network_allow` with an explicit allowlist

Allows download from a specific registry while blocking all other outbound connections. Postinstall scripts cannot reach attacker infrastructure.

```yaml
profiles:
  install:
    mode: sandbox
    network: on
    network_allow:
      - registry.npmjs.org
      - "*.npmjs.org"
```

**How enforcement works:**

- The container's DNS is pointed at a non-routable address (`192.0.2.1`), so arbitrary hostname lookups time out.
- Allowed hostnames are DNS-resolved on the host and injected into the container's `/etc/hosts` via `--add-host`.
- The container can reach those IPs directly; everything else is unreachable.

**Limitation:** raw IP connections bypass DNS filtering. A postinstall script that hardcodes an IP address can still connect if the network stack allows it. Package managers always use domain names, so in practice this catches the common exfiltration patterns.

### Glob and regex patterns

```yaml
network_allow:
  - registry.npmjs.org          # exact hostname
  - "*.npmjs.org"               # glob: expands to known subdomains
  - ".*\\.pypi\\.org"           # regex: same expansion logic
```

For known base domains, sbox expands the pattern to the full set of subdomains before resolving:

| Base domain | Expanded hosts |
|-------------|----------------|
| `npmjs.org` | `registry.npmjs.org`, `npmjs.org`, `www.npmjs.org` |
| `pypi.org` | `pypi.org`, `files.pythonhosted.org` |
| `crates.io` | `crates.io`, `static.crates.io` |
| `yarnpkg.com` | `registry.yarnpkg.com`, `yarnpkg.com` |
| `github.com` | `github.com`, `raw.githubusercontent.com`, `objects.githubusercontent.com` |

For unknown domains, only the base itself is resolved.

`sbox plan` shows what was resolved:

```
network_allow: [resolved] registry.npmjs.org=x.x.x.x, npmjs.org=x.x.x.x; [patterns] *.npmjs.org
```

### Registry allowlists by ecosystem

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
  - "*.pkg.go.dev"
```

## Option 3 — Two-phase install (strongest isolation)

Run the download and the script execution as separate steps with different network policies.

**Phase 1 — download without scripts** (network on, scripts disabled):

```bash
npm install --ignore-scripts
```

**Phase 2 — run scripts without network** (network off, no download):

```bash
npm rebuild   # or: npm run prepare
```

sbox does not orchestrate two-phase installs automatically today. You can model this with two dispatch rules and two profiles:

```yaml
profiles:
  download:
    mode: sandbox
    network: on
    network_allow:
      - "*.npmjs.org"

  scripts:
    mode: sandbox
    network: off
    writable: true

dispatch:
  npm-install-no-scripts:
    match:
      - "npm install --ignore-scripts*"
    profile: download

  npm-rebuild:
    match:
      - "npm rebuild*"
    profile: scripts
```

Then in your workflow:

```bash
sbox run -- npm install --ignore-scripts
sbox run -- npm rebuild
```

## What `network: off` actually blocks

With `network: off`, the container is started with `--network none`. Inside the container:

- DNS lookups fail immediately
- TCP/UDP connections to any IP fail with `ENETUNREACH`
- `curl`, `wget`, `fetch()`, raw sockets — all fail
- localhost connections also fail (no loopback interface)

Verified by the [adversarial test suite](../tests/adversarial/):

```
✓ HTTP exfil to attacker server    [BLOCKED]
✓ raw TCP socket to 1.1.1.1:443   [BLOCKED]
✓ curl to external URL             [BLOCKED]
✓ wget to external URL             [BLOCKED]
```
