# sbox Architecture

`sbox` is designed with a minimal, zero-config architecture to keep code maintainable, readable, and lightning-fast.

---

## Code Base Structure

```
src/
├── main.rs       # Entrypoint & CLI dispatch (~60 lines)
├── cli.rs        # Clap CLI definitions (~35 lines)
├── policy.rs     # Automatic policy detection (net ON/OFF, toolchain resolution) (~65 lines)
├── sandbox.rs    # Linux Landlock LSM & Network Namespace runner (~130 lines)
├── shim.rs       # Transparent shell shim generator & verifier (~55 lines)
├── platform.rs   # OS & path helpers (~20 lines)
└── error.rs      # Typed error definitions (~25 lines)
```

---

## Architectural Principles

1. **Zero Config First**: No `sbox.yaml` files required. `sbox` inspects the command and applies secure defaults automatically.
2. **Native Host Binary Execution**: Executes the developer's installed tools (`npm`, `cargo`, `uv`, `python`) directly without container engine overhead.
3. **No Heavy Design Patterns**: Code uses standard Rust idioms (`CommandExt`, `Landlock`, `clap`) without unnecessary abstractions or trait factories.
4. **Instant Startup**: Launches in < 2ms without Docker or Podman daemons.
