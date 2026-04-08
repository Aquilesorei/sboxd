# sbox documentation

Start here if you're new: **[How it works](how-it-works.md)** — explains bind mounts, what the sandbox actually isolates, and the network tension. Everything else makes more sense after reading this.

---

## Quick install

```bash
# macOS
brew tap aquilesorei/sbox && brew install sbox

# Linux
curl -fsSL https://github.com/Aquilesorei/sboxd/releases/latest/download/sbox-linux-x86_64 \
  -o ~/.local/bin/sbox && chmod +x ~/.local/bin/sbox

# Any platform
cargo install sboxd
```

---

## User guides

- [How it works](how-it-works.md) — bind mounts, what is isolated, what is not, the mental model
- [Getting started](getting-started.md) — install, create a config, run your first sandboxed command
- [Progressive adoption](adoption.md) — add sbox to an existing project one step at a time without breaking your workflow
- [Ecosystem guides](ecosystems.md) — ready-to-use configs for Node.js, Python, Rust, Go, PHP, Ruby
- [Network security](network.md) — the download/postinstall tension, `network_allow`, two-phase installs
- [Security model](security.md) — what sbox blocks, what it does not, adversarial test results
- [Shims](shims.md) — transparent interception, `sbox shim --verify`, Windows `.cmd` shims
- [Recipes](recipes.md) — CI pipelines, private registries, reusable sessions, keeping packages off the host
- [Troubleshooting](troubleshooting.md) — common errors, what causes them, how to fix them

## Reference

- [Config reference](config.md) — every `sbox.yaml` field explained

### Output formats

All commands support `--output-format text|json` as a global flag.

## Contributing

- [Architecture](architecture.md) — how the code is structured, execution flow, how to add features
- [Adversarial testing](adversarial-testing.md) — running the containment test suite against a real malicious package
