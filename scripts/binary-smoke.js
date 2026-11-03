import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const directory = mkdtempSync(path.join(tmpdir(), 'framer-next-binary-'));
const executable = path.resolve('dist/framer-next');
const environment = { PATH: '/usr/bin:/bin', HOME: directory };
const help = execFileSync(executable, ['--help'], {
  cwd: directory, env: environment, encoding: 'utf8',
});
assert.match(help, /Framer Next/);
const check = execFileSync(executable, ['check'], {
  cwd: directory, env: environment, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
});
assert.match(check, /^$/);
console.log('isolated executable smoke passed');
