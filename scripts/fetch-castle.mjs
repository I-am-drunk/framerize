import { spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const remote = 'git@github.com:I-am-drunk/framerize-castle.git';
const hostKey = 'github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl';

export function readRevision(text) {
  const revision = text.trim();
  if (!/^[0-9a-f]{40}$/.test(revision)) throw new Error('Castle revision must be a 40-character commit');
  return revision;
}

function redact(text) {
  return (text || '')
    .replace(/-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g, '[redacted key]')
    .split('\n').filter((line) => !line.includes('PRIVATE KEY')).join('\n').slice(-1200);
}

function git(args, cwd, env = {}) {
  const isolated = mkdtempSync(join(tmpdir(), 'castle-git-'));
  const config = join(isolated, 'config');
  writeFileSync(config, '');
  const childEnv = { ...process.env, GIT_CONFIG_GLOBAL: config, GIT_CONFIG_SYSTEM: config, ...env };
  delete childEnv.GIT_SSH_COMMAND;
  try {
    const result = spawnSync('git', args, { cwd, encoding: 'utf8', env: childEnv });
    if (result.status !== 0) throw new Error(`git ${args[0]} failed\n${redact(result.stderr)}`);
    return result.stdout.trim();
  } finally {
    rmSync(isolated, { recursive: true, force: true });
  }
}

function sshProgram() {
  if (process.platform !== 'win32') return 'ssh';
  const candidates = ['C:\\Windows\\System32\\OpenSSH\\ssh.exe', 'C:\\Program Files\\Git\\usr\\bin\\ssh.exe'];
  return candidates.find((candidate) => existsSync(candidate)) ?? 'ssh';
}

function sshEnv(keyText) {
  const dir = mkdtempSync(join(tmpdir(), 'castle-read-'));
  const keyPath = join(dir, 'key');
  const hosts = join(dir, 'known_hosts');
  writeFileSync(keyPath, keyText.endsWith('\n') ? keyText : `${keyText}\n`, { mode: 0o600 });
  chmodSync(keyPath, 0o600);
  writeFileSync(hosts, `${hostKey}\n`, { mode: 0o600 });
  const program = sshProgram();
  const wrapper = join(dir, process.platform === 'win32' ? 'ssh-castle.cmd' : 'ssh-castle');
  if (process.platform === 'win32') {
    writeFileSync(wrapper, `@echo off\r\n"${program}" -i "${keyPath}" -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile="${hosts}" %*\r\n`);
  } else {
    writeFileSync(wrapper, `#!/bin/sh\nexec ${JSON.stringify(program)} -i ${JSON.stringify(keyPath)} -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=${JSON.stringify(hosts)} "$@"\n`);
    chmodSync(wrapper, 0o700);
  }
  return { dir, env: { GIT_SSH: wrapper } };
}

export function fetchCastle(destination, revision, keyText) {
  let lastError;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    if (existsSync(join(destination, '.git'))) {
      try {
        if (git(['rev-parse', 'HEAD'], destination) === revision) return;
      } catch { /* replace a broken checkout */ }
    }
    if (existsSync(destination)) rmSync(destination, { recursive: true, force: true });
    const ssh = keyText ? sshEnv(keyText) : undefined;
    try {
      git(['clone', '--quiet', remote, destination], root, ssh?.env);
      git(['checkout', '--quiet', '--detach', revision], destination, ssh?.env);
      return;
    } catch (error) {
      lastError = error;
    } finally {
      if (ssh) rmSync(ssh.dir, { recursive: true, force: true });
    }
  }
  throw lastError;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const revision = readRevision(readFileSync(join(root, 'framerize-castle.rev'), 'utf8'));
  fetchCastle(join(root, 'framerize-castle'), revision, process.env.CASTLE_READ_KEY);
}
