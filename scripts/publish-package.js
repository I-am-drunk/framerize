import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const dist = path.resolve('dist');
const command = (name) => process.platform === 'win32'
  && process.env.FRAMER_NEXT_TEST_COMMAND ? `${process.cwd()}\\${name}.bat` : name;
const execute = (name, args, options = {}) => process.platform === 'win32'
  && process.env.FRAMER_NEXT_TEST_COMMAND
  ? execFileSync(process.env.ComSpec || 'cmd.exe',
    ['/d', '/s', '/c', name, ...args.map((arg) => arg.replace(/\^/g, '^^'))], options)
  : execFileSync(name, args, options);
const artifacts = readdirSync(dist).filter((name) => /^framer-next-.+\.tgz$/.test(name));
if (artifacts.length !== 1) throw new Error('dist must contain exactly one Framer Next tarball');
const version = artifacts[0].slice('framer-next-'.length, -'.tgz'.length);
const sums = path.join(dist, 'SHA256SUMS');
const tarball = path.join(dist, artifacts[0]);
const metadata = JSON.parse(readFileSync(path.join(dist, 'release.json'), 'utf8'));
const packageMetadata = JSON.parse(execute(command('tar'), ['-xOf', tarball,
  'package/package.json'], { encoding: 'utf8' }));
if (packageMetadata.name !== 'framer-next' || packageMetadata.version !== version) {
  throw new Error('tarball package metadata does not match its release filename');
}
if (!/^[a-f0-9]{64}$/.test(metadata.artifacts?.[artifacts[0]])) {
  throw new Error('release metadata does not contain the tarball SHA-256');
}
if (execute(command('git'), ['status', '--porcelain'], { encoding: 'utf8' }).trim()) {
  throw new Error('publication requires a clean source checkout');
}
if (metadata.version !== version || !/^[a-f0-9]{40}$/.test(metadata.commit)) {
  throw new Error('release artifacts have an invalid version or source commit');
}
try {
  execute(command('git'), ['cat-file', '-e', `${metadata.commit}^{commit}`], { stdio: 'ignore' });
} catch {
  throw new Error('release source commit is not present in this repository');
}
const run = (command, args, options = {}) => execute(command, args,
  { stdio: 'inherit', ...options });
try {
  execute(command('npm'), ['whoami', '--registry=https://registry.npmjs.org'], {
    stdio: ['ignore', 'ignore', 'pipe'],
  });
} catch {
  throw new Error('npm publication requires authenticated package-owner credentials');
}
try {
  execute(command('npm'), ['view', `framer-next@${version}`, 'version',
    '--registry=https://registry.npmjs.org'], { stdio: ['ignore', 'ignore', 'pipe'] });
  throw new Error(`framer-next@${version} is already published`);
} catch (error) {
  if (error.message === `framer-next@${version} is already published`) throw error;
  if (error.status !== 1 || !/(^|\s)E404(\s|$)/.test(error.stderr.toString())) {
    throw new Error('cannot verify whether this package version exists', { cause: error });
  }
}
const checksums = readFileSync(sums, 'utf8').trim().split('\n')
  .map((line) => {
    const artifact = line.match(/^([a-f0-9]{64}) [ *](.+)$/);
    if (!artifact) throw new Error('invalid SHA256SUMS entry');
    if (artifact[2] === artifacts[0] && artifact[1] !== metadata.artifacts[artifacts[0]]) {
      throw new Error('release metadata and SHA256SUMS tarball digests differ');
    }
    return line;
  })
  .filter((line) => {
    const artifact = line.split(/ [ *]/);
    return readdirSync(dist).includes(artifact.slice(1).join(' '));
  });
writeFileSync(path.join(dist, '.publish-SHA256SUMS'), `${checksums.join('\n')}\n`);
run(command('sha256sum'), ['-c', '.publish-SHA256SUMS'], { cwd: dist });
const provenance = process.env.GITHUB_ACTIONS === 'true' ? ['--provenance'] : [];
run(command('npm'), ['publish', tarball, '--ignore-scripts',
  ...provenance, '--access', 'public', '--registry=https://registry.npmjs.org',
  '--tag', version.includes('-') ? 'next' : 'latest']);
