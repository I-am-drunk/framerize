import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { fileURLToPath } from 'node:url';
import { createCastleMinter } from '../dist/castle.js';

const castleModule = fileURLToPath(new URL('../dist/castle.js', import.meta.url));

test('Castle mint returns a bounded token string', async () => {
  const minter = createCastleMinter();
  const { token } = await minter.mint({ signal: AbortSignal.timeout(30_000) });
  await minter.close();
  assert.equal(typeof token, 'string');
  assert.ok(token.length >= 64 && token.length <= 16_384);
});

test('Castle mint subprocess exits without lingering realm timers', async () => {
  const child = spawn(process.execPath, ['--input-type=module', '-e', `
    import { createCastleMinter } from ${JSON.stringify(castleModule)};
    const minter = createCastleMinter();
    await minter.mint({ signal: AbortSignal.timeout(30_000) });
    await minter.close();
  `], { stdio: ['ignore', 'pipe', 'pipe'] });
  let stderr = '';
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  const deadline = setTimeout(() => child.kill('SIGKILL'), 15_000);
  const [code, signal] = await once(child, 'exit');
  clearTimeout(deadline);
  assert.equal(signal, null, stderr);
  assert.equal(code, 0, stderr);
});
