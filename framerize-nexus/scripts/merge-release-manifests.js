import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { copyFileSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const directory = path.resolve(process.env.RELEASE_DIST || 'dist');
const digest = (file) => createHash('sha256').update(readFileSync(file)).digest('hex');
const artifacts = {};
const builds = [];
let identity;
let licenses;
const expectedVersion = JSON.parse(readFileSync('package.json', 'utf8')).version;
const expectedCommit = process.env.GITHUB_SHA ||
  execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
for (const platform of ['linux-x64', 'macos-arm64', 'windows-x64']) {
  const source = path.join(directory, platform);
  const metadata = JSON.parse(readFileSync(path.join(source, 'release.json'), 'utf8'));
  identity ??= { version: metadata.version, commit: metadata.commit };
  if (!/^[a-f0-9]{40}$/.test(metadata.commit)) throw new Error('invalid release source commit');
  if (metadata.version !== expectedVersion || (expectedCommit && metadata.commit !== expectedCommit)) {
    throw new Error('release identity does not match the reviewed checkout');
  }
  if (metadata.version !== identity.version || metadata.commit !== identity.commit) {
    throw new Error(`release identity mismatch for ${platform}`);
  }
  const executable = platform === 'windows-x64' ? 'framerize-nexus.exe' : 'framerize-nexus';
  const archive = `framerize-nexus-${platform}.${platform === 'windows-x64' ? 'zip' : 'tar.gz'}`;
  const expected = new Map([[executable, metadata.artifacts?.[executable]],
    [archive, metadata.artifacts?.[archive]]]);
  if (Object.keys(metadata.artifacts || {}).length !== expected.size) {
    throw new Error(`unexpected build artifacts for ${platform}`);
  }
  licenses ??= metadata.licenses;
  if (JSON.stringify(metadata.licenses) !== JSON.stringify(licenses)) {
    throw new Error(`release licenses differ for ${platform}`);
  }
  for (const name of ['NODE-LICENSE', 'LICENSE']) {
    if (!/^[a-f0-9]{64}$/.test(licenses?.[name]) || digest(path.join(source, name)) !== licenses[name]) {
      throw new Error(`release license digest mismatch for ${platform}/${name}`);
    }
  }
  for (const [name, checksum] of expected) {
    if (!/^[a-f0-9]{64}$/.test(checksum) || digest(path.join(source, name)) !== checksum) {
      throw new Error(`release digest mismatch for ${platform}/${name}`);
    }
  }
  copyFileSync(path.join(source, archive), path.join(directory, archive));
  artifacts[archive] = expected.get(archive);
  builds.push({ platform, node: metadata.node, artifacts: metadata.artifacts });
}
const tarball = `framerize-nexus-${identity.version}.tgz`;
artifacts[tarball] = digest(path.join(directory, tarball));
copyFileSync(path.join(directory, 'linux-x64', 'NODE-LICENSE'), path.join(directory, 'NODE-LICENSE'));
copyFileSync(path.join(directory, 'linux-x64', 'LICENSE'), path.join(directory, 'LICENSE'));
for (const name of ['NODE-LICENSE', 'LICENSE']) artifacts[name] = licenses[name];
const entries = Object.entries(artifacts).map(([name, checksum]) => `${checksum}  ${name}`);
writeFileSync(path.join(directory, 'SHA256SUMS'), `${entries.join('\n')}\n`);
writeFileSync(path.join(directory, 'release.json'), `${JSON.stringify({
  ...identity, artifacts, builds,
}, null, 2)}\n`);
