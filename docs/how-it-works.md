# How sbox works

Before reading the other docs, it helps to have a clear mental model of what sbox actually does. A lot of the "why does this behave that way?" questions have the same answer: bind mounts.

---

## The container is not a black box

When sbox runs `npm install` in a container, your project directory is **bind-mounted** into it. A bind mount means the container is not working with a copy — it is reading and writing directly to your host filesystem through a shared path.

```
Host filesystem:               Container filesystem:
~/myproject/              →    /workspace/                (read-only)
~/myproject/node_modules/ →    /workspace/node_modules/   (read-write)
```

npm inside the container sees `/workspace/node_modules/`. It writes packages there. Because it is a bind mount, those writes land at `~/myproject/node_modules/` on your host. When the container exits, the files stay there — the container is gone but the output is not.

This is why `npm run build` on the host works after a sandboxed install. The host `node_modules/` is the same directory that was written inside the container. No copying, no export step.

---

## What the sandbox actually isolates

The container gives npm install a **different view of the world**:

| Resource | What the container sees | What that prevents |
|----------|------------------------|-------------------|
| Filesystem | Only the workspace (read-only except `writable_paths`) | Postinstall can't read `~/.ssh/`, `~/.aws/`, or your source tree outside the workspace |
| Network | Nothing, or only allowed registries | Postinstall can't phone home to an attacker server |
| Environment | Only explicitly forwarded vars | Postinstall can't read `NPM_TOKEN`, `AWS_SECRET_ACCESS_KEY`, etc. from the host env |
| User | Same UID as you (via `keep-id`) | Files written to `node_modules/` are owned by you, not root |

The container cannot see or reach anything that isn't explicitly mounted or forwarded. That is the whole model.

---

## The workspace is read-only by default

`workspace.writable: false` means the entire project directory is mounted read-only inside the container. npm cannot modify your source files, your `package.json`, your `.git/` directory, or anything else.

The only writable area is `writable_paths`:

```yaml
workspace:
  writable: false
  writable_paths:
    - node_modules   # npm can write here, nowhere else
```

If a `writable_paths` entry points to a file such as `package-lock.json`, `uv.lock`, or `Cargo.lock`, sbox opens the parent directory for writes rather than mounting only that file. That preserves normal lockfile update semantics while the rest of the workspace stays read-only.

If a postinstall script tries to write a backdoor to `.git/hooks/pre-commit`, it gets `EACCES`. If it tries to modify `package.json`, it gets `EROFS`.

---

## The partial adoption trade-off

If you sandbox the install but run everything else on the host:

```bash
sbox run -- npm install   # sandboxed — postinstall scripts contained
npm run build             # on host — reading from ./node_modules/
```

This works because `node_modules/` was written by the container to your host filesystem via the bind mount.

But there is a residual risk: **whatever is in `node_modules/` now runs on your host with your full privileges** when you call `npm run build`. A malicious package could have planted code in `node_modules/.bin/` that executes during your build step outside the sandbox.

The sandbox contained the postinstall scripts during install. It did not contain what was installed.

To close this gap, route execution through sbox too:

```bash
sbox run -- npm install     # postinstall scripts contained
sbox run -- npm run build   # build scripts also contained
sbox run -- node server.js  # runtime also contained
```

See [Progressive adoption](adoption.md) for how to get there without breaking everything at once.

---

## Network: the hardest part

`npm install` needs to download packages. Postinstall scripts should not have the network. These two requirements apply to the same container at the same time.

If you use `network: off`, npm cannot download anything. If you use `network: on`, postinstall scripts can reach the internet.

The practical answer is `network_allow` — restrict the container's DNS to a specific registry so package downloads work but arbitrary outbound connections fail:

```yaml
profiles:
  install:
    network: on
    network_allow:
      - "*.npmjs.org"
```

How this works: sbox points the container's DNS resolver at a non-routable address (`192.0.2.1`), so any hostname lookup that is not in the allowlist times out. For allowed hostnames, sbox resolves them on the host and injects the IPs directly into the container's `/etc/hosts`. npm can reach `registry.npmjs.org` because its IP is already known. `evil.attacker.com` never resolves.

The remaining gap: a hardcoded attacker IP in a postinstall script bypasses DNS entirely. `network_allow` is not a firewall. For complete network isolation, use `network: off` with a pre-populated cache or a local registry mirror. See [Network security](network.md) for all the options.

---

## `sbox plan` — inspect before you run

Every sandboxed command goes through a resolution phase that produces an `ExecutionPlan`. `sbox plan` shows you that plan without executing anything:

```bash
sbox plan -- npm install
```

You see: which image, which mounts, which environment variables pass through, which are denied, what network policy applies, which profile was selected, and why. This is useful both for debugging broken configs and for security review — you can audit exactly what the sandbox will do before it does it.

---

## Reusable sessions

By default, sbox starts a new container for each command and removes it when done. This adds ~0.5–1 second of startup overhead per invocation.

With `reuse_container: true`, the container stays running between commands. Subsequent `sbox run` calls use `podman exec` into the live container instead of starting a new one. Startup drops to near zero.

```yaml
runtime:
  reuse_container: true
```

The trade-off: the container accumulates state between runs. If a postinstall script wrote something into `/tmp` inside the container, it persists until you run `sbox clean`. For install commands you probably want a fresh container each time. For build/run commands where startup latency matters, reuse is useful.
