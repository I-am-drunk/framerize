import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { artifactNames, currentTarget, digest, sourceIdentity, verifyLicenses, writeSums } from './release-artifacts.js';

const identity = sourceIdentity();
const target = currentTarget();
const [executable, archive] = artifactNames(target);
const build = JSON.parse(readFileSync('dist/build.json', 'utf8'));
const tarball = `${identity.name}-${identity.version}.tgz`;
if (Object.entries(identity).some(([key, value]) => build[key] !== value) ||
    build.target !== target || build.node !== process.version ||
    build.packageSha256 !== digest(tarball) || build.packageBytes !== readFileSync(tarball).length ||
    build.executableSha256 !== digest(`dist/${executable}`)) throw new Error('Stale executable build');
const license = process.env.NODE_DISTRIBUTION_LICENSE
  ? path.resolve(process.env.NODE_DISTRIBUTION_LICENSE)
  : [path.join(path.dirname(process.execPath), 'LICENSE'),
    path.join(path.dirname(process.execPath), '..', 'LICENSE')].find(existsSync);
if (!license || !existsSync(license)) throw new Error('Node distribution LICENSE is required');
const normalize = (file) => readFileSync(file, 'utf8').replace(/\r\n/g, '\n');
writeFileSync('dist/NODE-LICENSE', normalize(license));
writeFileSync('dist/LICENSE', normalize(fileURLToPath(new URL('../LICENSE', import.meta.url))));
verifyLicenses('dist');
if (process.platform === 'win32') {
  execFileSync('powershell', ['-NoLogo', '-NoProfile', '-Command',
    `Compress-Archive -LiteralPath 'dist/${executable}', 'dist/LICENSE', 'dist/NODE-LICENSE' -DestinationPath 'dist/${archive}' -Force`],
  { stdio: 'inherit' });
} else {
  execFileSync('tar', ['-czf', `dist/${archive}`, '-C', 'dist', executable, 'LICENSE', 'NODE-LICENSE'],
    { stdio: 'inherit' });
}
writeSums('dist', Object.fromEntries([executable, archive].map((name) => [name, digest(`dist/${name}`)])));
process.stdout.write(`Packaged dist/${archive}\n`);
