#!/usr/bin/env bash
# sbox adversarial test harness
#
# Installs a malicious npm package inside a sbox sandbox and verifies that
# common postinstall attack patterns were blocked.
#
# USAGE:
#   ./tests/adversarial/run.sh [--only <category>] [--package <path.tgz>]
#
# REQUIREMENTS:
#   - Run on a VM or disposable machine (snapshot first)
#   - Rootless Podman installed
#   - sbox binary in PATH

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
WORK_DIR="$(mktemp -d /tmp/sbox-adversarial-XXXX)"
ONLY=""
PACKAGE=""

# ── Parse args ────────────────────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
  case "$1" in
    --only)    ONLY="$2";    shift 2 ;;
    --package) PACKAGE="$2"; shift 2 ;;
    *)         echo "unknown arg: $1"; exit 1 ;;
  esac
done

trap 'rm -rf "$WORK_DIR"' EXIT

# ── Setup ─────────────────────────────────────────────────────────────────────

echo "sbox adversarial test"
echo "work dir: $WORK_DIR"
echo ""

# Copy test workspace into temp dir
cp "$SCRIPT_DIR/sbox.yaml" "$WORK_DIR/sbox.yaml"
mkdir -p "$WORK_DIR/node_modules"
# Minimal package.json so npm doesn't try to create one (workspace is read-only)
echo '{"name":"sbox-adversarial-workspace","version":"1.0.0","private":true}' \
  > "$WORK_DIR/package.json"

# Build the evil package tarball if no custom package provided
if [[ -z "$PACKAGE" ]]; then
  echo "► packaging evil-package..."
  cd "$SCRIPT_DIR/evil-package"
  PKG_FILE="$(npm pack --pack-destination "$WORK_DIR" 2>/dev/null | tail -1)"
  PACKAGE="$WORK_DIR/$PKG_FILE"
  # Container path: workspace is mounted at /workspace inside the container
  PACKAGE_CONTAINER="/workspace/$PKG_FILE"
  cd "$WORK_DIR"
else
  PKG_FILE="$(basename "$PACKAGE")"
  cp "$PACKAGE" "$WORK_DIR/$PKG_FILE"
  PACKAGE="$WORK_DIR/$PKG_FILE"
  PACKAGE_CONTAINER="/workspace/$PKG_FILE"
fi

echo "► package: $PACKAGE"
echo ""

# Plant fake credential files so we can detect reads
mkdir -p "$WORK_DIR/.ssh" "$WORK_DIR/.aws" "$WORK_DIR/.docker"
echo "FAKE_SSH_PRIVATE_KEY_CONTENT" > "$WORK_DIR/.ssh/id_ed25519"
echo "FAKE_NPMRC_TOKEN=secret123"   > "$WORK_DIR/.npmrc"
echo "FAKE_NETRC_PASSWORD=secret"   > "$WORK_DIR/.netrc"
mkdir -p "$WORK_DIR/.aws"
printf "[default]\naws_access_key_id=FAKEKEYID\naws_secret_access_key=FAKESECRET\n" > "$WORK_DIR/.aws/credentials"

# ── Run sandboxed install ─────────────────────────────────────────────────────

echo "► running sandboxed npm install..."
RAW_OUTPUT=""
set +e
RAW_OUTPUT=$(cd "$WORK_DIR" && sbox run -- npm install --no-save --no-package-lock --ignore-scripts=false "$PACKAGE_CONTAINER" 2>&1)
INSTALL_EXIT=$?
set -e

echo "install exit code: $INSTALL_EXIT"
echo ""

# Read results written by postinstall.js to node_modules (the only writable path).
# npm buffers/suppresses lifecycle script output, so file-based IPC is more reliable.
RESULTS_FILE="$WORK_DIR/node_modules/.sbox-adversarial-results.json"
RESULTS_JSON=""
if [[ -f "$RESULTS_FILE" ]]; then
  RESULTS_JSON="$(cat "$RESULTS_FILE")"
fi

if [[ -z "$RESULTS_JSON" ]]; then
  echo "ERROR: no results file from postinstall script"
  echo "--- raw output ---"
  echo "$RAW_OUTPUT"
  exit 1
fi

# ── Evaluate results ──────────────────────────────────────────────────────────

PASS=0
FAIL=0
SKIP=0

check() {
  local name="$1"
  local expected="$2"  # BLOCKED or SUCCESS

  # Filter by --only if set
  if [[ -n "$ONLY" ]] && [[ "$name" != *"$ONLY"* ]]; then
    ((SKIP++)) || true
    return
  fi

  local status
  status=$(echo "$RESULTS_JSON" | \
    python3 -c "
import json, sys
data = json.load(sys.stdin)
entry = next((r for r in data if r['name'] == '$name'), None)
if entry is None:
    print('MISSING')
else:
    print(entry['status'])
" 2>/dev/null || echo "MISSING")

  local value
  value=$(echo "$RESULTS_JSON" | \
    python3 -c "
import json, sys
data = json.load(sys.stdin)
entry = next((r for r in data if r['name'] == '$name'), None)
if entry:
    print(entry.get('value', entry.get('error', '')))
" 2>/dev/null || echo "")

  if [[ "$status" == "$expected" ]]; then
    printf "  ✓ PASS  %-45s [%s]\n" "$name" "$status"
    ((PASS++)) || true
  elif [[ "$status" == "MISSING" ]]; then
    printf "  ? SKIP  %-45s [not reported]\n" "$name"
    ((SKIP++)) || true
  else
    printf "  ✗ FAIL  %-45s [%s] %s\n" "$name" "$status" "$value"
    ((FAIL++)) || true
  fi
}

echo "── Credential reads (expect: BLOCKED) ──────────────────────────────"
check "read ~/.ssh/id_ed25519"      BLOCKED
check "read ~/.ssh/id_rsa"          BLOCKED
check "read ~/.npmrc"               BLOCKED
check "read ~/.netrc"               BLOCKED
check "read ~/.aws/credentials"     BLOCKED
check "read ~/.docker/config.json"  BLOCKED
check "read ~/.kube/config"         BLOCKED

echo ""
echo "── Environment leaks (expect: BLOCKED) ─────────────────────────────"
check "dump sensitive env vars"     BLOCKED

echo ""
echo "── Network exfiltration (expect: BLOCKED) ──────────────────────────"
check "HTTP exfil to attacker server"    BLOCKED
check "raw TCP socket to 1.1.1.1:443"   BLOCKED
check "curl to external URL"             BLOCKED
check "wget to external URL"             BLOCKED

echo ""
echo "── Workspace writes (expect: BLOCKED) ──────────────────────────────"
check "write to workspace root (../../../)"  BLOCKED
check "write to .git/hooks/pre-commit"       BLOCKED

echo ""
echo "── Privilege escalation (expect: BLOCKED) ──────────────────────────"
check "sudo id"                  BLOCKED
check "check if running as root" BLOCKED
check "read /etc/shadow"         BLOCKED

echo ""
echo "────────────────────────────────────────────────────────────────────"
printf "Results: %d passed, %d failed, %d skipped\n" "$PASS" "$FAIL" "$SKIP"
echo ""

if [[ "$FAIL" -gt 0 ]]; then
  echo "CONTAINMENT FAILURES DETECTED — review the output above."
  echo "If running on a real machine (not a VM), assume it is compromised."
  exit 1
else
  echo "All checks passed — sandbox held."
  exit 0
fi
