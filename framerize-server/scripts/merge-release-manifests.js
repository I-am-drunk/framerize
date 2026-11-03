import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { artifactNames, digest, sourceIdentity, targets, verifyLicenses, verifySums, writeSums } from './release-artifacts.js';

const directory = path.resolve(process.env.RELEASE_DIST || 'dist');
const identity = sourceIdentity();
const tarball = `${identity.name}-${identity.version}.tgz`;
const common = [tarball, 'LICENSE', 'NODE-LICENSE'];
const artifacts = Object.fromEntries([[tarball, digest(path.join(directory, tarball))]]);
const builds = [];
let node;
for (const target of Object.keys(targets)) {
  const source = path.join(directory, target);
  const metadata = JSON.parse(readFileSync(path.join(source, 'release.json'), 'utf8'));
  if (Object.entries(identity).some(([key, value]) => metadata[key] !== value) || metadata.target !== target) {
    throw new Error(`Release identity mismatch for ${target}`);
  }
  node ??= metadata.node;
  const version = /^v(\d+)\.(\d+)\.\d+$/.exec(node);
  if (!version || Number(version[1]) < 22 || (Number(version[1]) === 22 && Number(version[2]) < 12) ||
      metadata.node !== node) {
    throw new Error('Release Node versions differ or are unsupported');
  }
  const expected = [...artifactNames(target), ...common].sort();
  if (JSON.stringify(Object.keys(metadata.artifacts ?? {}).sort()) !== JSON.stringify(expected)) {
    throw new Error(`Unexpected build artifacts for ${target}`);
  }
  verifySums(source, metadata.artifacts);
  if (metadata.packageSha256 !== metadata.artifacts[tarball] ||
      metadata.packageBytes !== readFileSync(path.join(source, tarball)).length) {
    throw new Error('Native build package binding mismatch');
  }
  verifyLicenses(source);
  for (const name of common) {
    artifacts[name] ??= metadata.artifacts[name];
    if (artifacts[name] !== metadata.artifacts[name]) throw new Error(`Shared artifact mismatch: ${name}`);
  }
  builds.push(metadata);
}
const output = path.join(directory, 'qualified');
mkdirSync(output, { recursive: true });
for (const build of builds) {
  const archive = artifactNames(build.target)[1];
  copyFileSync(path.join(directory, build.target, archive), path.join(output, archive));
  artifacts[archive] = build.artifacts[archive];
}
copyFileSync(path.join(directory, tarball), path.join(output, tarball));
for (const name of ['LICENSE', 'NODE-LICENSE']) {
  copyFileSync(path.join(directory, 'linux-x64', name), path.join(output, name));
}
writeSums(output, artifacts);
writeFileSync(path.join(output, 'release.json'), `${JSON.stringify({ ...identity, artifacts, builds }, null, 2)}\n`);
verifySums(output, artifacts);
process.stdout.write(`Verified three platform manifests for ${identity.commit}\n`);
