import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const serverRoot = fileURLToPath(new URL('..', import.meta.url));

const mintScript = `
import { createCastleMinter } from './dist/castle.js';
const minter = createCastleMinter();
try {
  const { token } = await minter.mint({ signal: AbortSignal.timeout(25_000) });
  if (typeof token !== 'string' || token.length < 64) throw new Error('unexpected mint output');
} finally {
  await minter.close();
}
`;

test('castle mint reaps realm timers so an isolated subprocess can exit', { timeout: 15_000 }, () => {
  const result = spawnSync(process.execPath, ['--input-type=module', '-e', mintScript], {
    cwd: serverRoot,
    encoding: 'utf8',
    timeout: 12_000,
    env: { ...process.env, NODE_OPTIONS: '' },
  });
  assert.equal(result.error, undefined, result.error?.message);
  assert.equal(result.signal, null, result.signal);
  assert.equal(result.status, 0, result.stderr || result.stdout);
});
