# Fedora / Podman Signature Policy Example

This directory contains a minimal user-level containers policy example for Fedora and Podman.

Files:

- `policy.json`: strict default reject policy with a signed image scope example
- `registries.d/example.yaml`: lookaside signature configuration example for simple-signing workflows

These files are examples only. Replace the placeholder registry names, key paths, and lookaside URLs with values from your environment.

Typical Fedora setup:

```bash
mkdir -p ~/.config/containers
mkdir -p ~/.config/containers/registries.d
cp examples/fedora-podman-signature-policy/policy.json ~/.config/containers/policy.json
cp examples/fedora-podman-signature-policy/registries.d/example.yaml ~/.config/containers/registries.d/example.yaml
```

Then edit:

- `registry.example.com/team/secure-images`
- `/home/YOU/.config/containers/keys/team-signing.gpg`
- `https://registry.example.com/lookaside`

After that, verify readiness:

```bash
sbox doctor
```

If your `sbox.yaml` also sets `image.verify_signature: true`, `sbox run` will fail until the policy and signatures are valid.
