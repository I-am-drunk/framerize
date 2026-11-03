import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const directory = path.resolve('dist');
const readJson = (name) => JSON.parse(readFileSync(path.join(directory, name), 'utf8'));
const metadata = readJson('release.json');
const artifacts = { ...metadata.artifacts };
for (const line of readFileSync(path.join(directory, 'SHA256SUMS'), 'utf8').trim().split('\n')) {
  const match = line.match(/^([a-f0-9]{64}) [ *](.+)$/);
  if (!match) throw new Error('invalid SHA256SUMS entry');
  if (artifacts[match[2]] && artifacts[match[2]] !== match[1]) {
    throw new Error(`conflicting release digests for ${match[2]}`);
  }
  artifacts[match[2]] = match[1];
}
for (const [name, digest] of Object.entries(artifacts)) {
  const actual = createHash('sha256').update(readFileSync(path.join(directory, name))).digest('hex');
  if (actual !== digest) throw new Error(`release digest mismatch for ${name}`);
}
const entries = Object.entries(artifacts).map(([name, digest]) => `${digest}  ${name}`);
writeFileSync(path.join(directory, 'SHA256SUMS'), `${entries.join('\n')}\n`);
writeFileSync(path.join(directory, 'release.json'), `${JSON.stringify({ ...metadata, artifacts }, null, 2)}\n`);
