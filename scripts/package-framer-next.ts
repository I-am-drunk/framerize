import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { chmodSync, closeSync, copyFileSync, constants, fstatSync, lstatSync, mkdirSync,
  openSync, readFileSync, readSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { accountTemplate, recoveryReadme } from './framer-next-content.js';

interface NativeRelease {
  name: string; version: string; commit: string; target: string; node: string;
  packageSha256: string; packageBytes: number; artifacts: Record<string, string>;
}
const hashPattern = /^[a-f0-9]{64}$/;
const digest = (data: Buffer | string) => createHash('sha256').update(data).digest('hex');
const targets = ['linux-x64', 'macos-arm64', 'windows-x64'];
const binaryLimit = 256 * 1024 * 1024;

function fileHash(file: string, limit: number) {
  assert.ok(lstatSync(file).isFile(), 'Recovery input must be a regular file');
  const handle = openSync(file, 'r');
  try {
    const info = fstatSync(handle);
    assert.ok(info.isFile() && info.size > 0 && info.size <= limit, 'Invalid recovery input size');
    const buffer = Buffer.alloc(65536); const hash = createHash('sha256');
    let total = 0; let count: number;
    while ((count = readSync(handle, buffer, 0, buffer.length, null)) > 0) {
      total += count; assert.ok(total <= limit, 'Recovery input grew beyond its limit');
      hash.update(buffer.subarray(0, count));
    }
    assert.equal(total, info.size, 'Recovery input changed size');
    return { bytes: total, sha256: hash.digest('hex') };
  } finally { closeSync(handle); }
}

export function createFramerNext(input: string, manifestSha256: string, output: string) {
  assert.match(manifestSha256, hashPattern);
  assert.ok(lstatSync(input).isDirectory(), 'Native input must be a directory');
  const manifestPath = join(input, 'release.json');
  assert.equal(fileHash(manifestPath, 65536).sha256, manifestSha256, 'Native manifest differs');
  const manifestBytes = readFileSync(manifestPath);
  assert.equal(digest(manifestBytes), manifestSha256);
  const native = JSON.parse(manifestBytes.toString('utf8')) as NativeRelease;
  assert.equal(native.name, 'framerize');
  assert.match(native.version, /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/);
  assert.match(native.commit, /^[a-f0-9]{40}$/);
  assert.match(native.node, /^v\d+\.\d+\.\d+$/);
  assert.ok(targets.includes(native.target), 'Unsupported recovery target');
  const executable = 'framerize-server' + (native.target === 'windows-x64' ? '.exe' : '');
  const archive = `framerize-server-${native.target}.${native.target === 'windows-x64' ? 'zip' : 'tar.gz'}`;
  const packageName = `framerize-${native.version}.tgz`;
  const copied = [executable, 'LICENSE', 'NODE-LICENSE'];
  assert.deepEqual(Object.keys(native.artifacts).sort(), [...copied, archive, packageName].sort());
  for (const hash of Object.values(native.artifacts)) assert.match(hash, hashPattern);
  assert.equal(native.artifacts[packageName], native.packageSha256);
  assert.ok(Number.isSafeInteger(native.packageBytes) && native.packageBytes > 0);
  const inventory: Record<string, { bytes: number; sha256: string }> = {};
  for (const name of copied) {
    inventory[name] = fileHash(join(input, name), name === executable ? binaryLimit : 1024 * 1024);
    assert.equal(inventory[name].sha256, native.artifacts[name], 'Native payload differs');
  }
  mkdirSync(output, { mode: 0o700 });
  try {
    for (const name of copied) {
      copyFileSync(join(input, name), join(output, name), constants.COPYFILE_EXCL);
      assert.deepEqual(fileHash(join(output, name), binaryLimit), inventory[name]);
      chmodSync(join(output, name), name === executable ? 0o755 : 0o600);
    }
    const generated = new Map<string, Buffer>([
      ['accounts.example.json', Buffer.from(accountTemplate)],
      ['README.md', Buffer.from(recoveryReadme(executable))],
      ['native-release.json', manifestBytes],
    ]);
    for (const [name, data] of generated) inventory[name] = { bytes: data.length, sha256: digest(data) };
    const recovery = { name: 'Framer Next', formatVersion: 1, scope: 'native-recovery-folder',
      version: native.version, sourceCommit: native.commit, target: native.target, node: native.node,
      nativeManifestSha256: manifestSha256, files: inventory };
    const receipt = Buffer.from(JSON.stringify(recovery, null, 2) + '\n');
    generated.set('recovery.json', receipt);
    const sums = { ...inventory, 'recovery.json': { bytes: receipt.length, sha256: digest(receipt) } };
    generated.set('SHA256SUMS', Buffer.from(Object.entries(sums)
      .map(([name, info]) => `${info.sha256}  ${name}\n`).join('')));
    for (const [name, data] of generated) {
      writeFileSync(join(output, name), data, { flag: 'wx', mode: 0o600 });
    }
    return { ...recovery, output: resolve(output) };
  } catch (error) { rmSync(output, { recursive: true, force: true }); throw error; }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length === 3 && process.argv[2] === '--help') {
      console.log('Usage: package-framer-next NATIVE_DIR MANIFEST_SHA256 NEW_OUTPUT_DIR');
      console.log('Copies verified native bytes and a disabled account template. Never starts the server.');
    } else {
      assert.equal(process.argv.length, 5);
      const result = createFramerNext(process.argv[2]!, process.argv[3]!, process.argv[4]!);
      console.log(JSON.stringify(result));
    }
  } catch {
    console.error('Framer Next recovery packaging failed');
    process.exitCode = 1;
  }
}
