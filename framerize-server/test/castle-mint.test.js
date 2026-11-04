import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createCastleMinter } from '../dist/castle.js';
import { createServerMinter } from '../dist/castle-loader.js';

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const castleModule = pathToFileURL(path.join(packageRoot, 'dist/castle.js')).href;
const mintTimeoutMs = 120_000;

test('createCastleMinter returns an inline comma-delimited castle token', { timeout: mintTimeoutMs }, async (t) => {
  const minter = createCastleMinter();
  t.after(() => minter.close());
  const { token } = await minter.mint({ signal: AbortSignal.timeout(mintTimeoutMs) });
  assert.equal(typeof token, 'string');
  assert.ok(token.length >= 64 && token.length <= 16_384);
  assert.ok(token.includes(','), 'inline castle tokens are comma-delimited segments');
});

test('createCastleMinter rejects overlapping mint and use after close', { timeout: mintTimeoutMs }, async (t) => {
  const minter = createCastleMinter();
  t.after(() => minter.close());
  const active = minter.mint({ signal: AbortSignal.timeout(mintTimeoutMs) });
  await assert.rejects(
    minter.mint({ signal: AbortSignal.timeout(1000) }),
    (error) => error instanceof Error && error.message === 'busy',
  );
  await active;
  await minter.close();
  await assert.rejects(
    minter.mint({ signal: AbortSignal.timeout(1000) }),
    (error) => error instanceof Error && error.message === 'closed',
  );
});

test('createServerMinter rejects an already-aborted startup signal', async () => {
  const signal = AbortSignal.abort(new Error('synthetic-private'));
  await assert.rejects(createServerMinter(signal), { message: 'synthetic-private' });
});

test('completed mint exits without realm timers keeping the event loop alive', () => {
  const result = spawnSync(process.execPath, [
    '--input-type=module',
    '-e',
    `import { createCastleMinter } from ${JSON.stringify(castleModule)};
const minter = createCastleMinter();
await minter.mint({ signal: AbortSignal.timeout(${mintTimeoutMs}) });
await minter.close();`,
  ], { cwd: packageRoot, encoding: 'utf8', timeout: mintTimeoutMs + 5_000 });
  assert.equal(result.status, 0, result.stderr || result.stdout);
});
