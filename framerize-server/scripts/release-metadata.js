import { copyFileSync, readFileSync, writeFileSync } from 'node:fs';
import { artifactNames, currentTarget, digest, sourceIdentity, verifyLicenses, verifySums, writeSums } from './release-artifacts.js';

const identity = sourceIdentity();
const target = currentTarget();
const [executable, archive] = artifactNames(target);
const build = JSON.parse(readFileSync('dist/build.json', 'utf8'));
if (Object.entries(identity).some(([key, value]) => build[key] !== value) ||
    build.target !== target || build.node !== process.version ||
    build.executableSha256 !== digest(`dist/${executable}`)) throw new Error('Stale executable build');
verifySums('dist', Object.fromEntries([executable, archive].map((name) => [name, digest(`dist/${name}`)])));
const tarball = `${identity.name}-${identity.version}.tgz`;
copyFileSync(tarball, `dist/${tarball}`);
const artifacts = Object.fromEntries([executable, archive, tarball, 'LICENSE', 'NODE-LICENSE']
  .map((name) => [name, digest(`dist/${name}`)]));
verifyLicenses('dist');
writeSums('dist', artifacts);
writeFileSync('dist/release.json', `${JSON.stringify({ ...identity, target, node: build.node, artifacts }, null, 2)}\n`);
process.stdout.write(`Recorded ${target} artifacts for ${identity.commit}\n`);
