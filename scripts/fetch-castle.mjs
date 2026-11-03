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

function git(args, cwd, env = {}) {
  const isolated = mkdtempSync(join(tmpdir(), 'castle-git-'));
  const config = join(isolated, 'config');
  writeFileSync(config, '');
  try {
    const result = spawnSync('git', args, {
      cwd, encoding: 'utf8',
      env: { ...process.env, GIT_CONFIG_GLOBAL: config, GIT_CONFIG_SYSTEM: config, ...env },
    });
    if (result.status !== 0) throw new Error(`git ${args[0]} failed`);
    return result.stdout.trim();
  } finally {
    rmSync(isolated, { recursive: true, force: true });
  }
}

function sshEnv(keyText) {
  const dir = mkdtempSync(join(tmpdir(), 'castle-read-'));
  const keyPath = join(dir, 'key');
  const hosts = join(dir, 'known_hosts');
  writeFileSync(keyPath, keyText.endsWith('\n') ? keyText : `${keyText}\n`, { mode: 0o600 });
  chmodSync(keyPath, 0o600);
  writeFileSync(hosts, `${hostKey}\n`, { mode: 0o600 });
  return {
    dir,
    env: { GIT_SSH_COMMAND: `ssh -i ${keyPath} -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=${hosts}` },
  };
}

export function fetchCastle(destination, revision, keyText) {
  if (existsSync(join(destination, '.git'))) {
    const current = git(['rev-parse', 'HEAD'], destination);
    if (current === revision) return;
    rmSync(destination, { recursive: true, force: true });
  } else if (existsSync(destination)) rmSync(destination, { recursive: true, force: true });
  const ssh = keyText ? sshEnv(keyText) : undefined;
  try {
    git(['clone', '--quiet', remote, destination], root, ssh?.env);
    git(['checkout', '--quiet', '--detach', revision], destination, ssh?.env);
  } finally {
    if (ssh) rmSync(ssh.dir, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const revision = readRevision(readFileSync(join(root, 'framerize-castle.rev'), 'utf8'));
  fetchCastle(join(root, 'framerize-castle'), revision, process.env.CASTLE_READ_KEY);
}
