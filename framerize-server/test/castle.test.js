import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createCastleMinter } from '../src/castle.js';

function makeDir(t, prefix) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), prefix));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return directory;
}


function emptyTrace(t) {
  const directory = makeDir(t, 'castle-trace-');
  const file = path.join(directory, 'trace.json');
  fs.writeFileSync(file, JSON.stringify({ events: [] }));
  return file;
}

const ECHO_NODE = `let data = '';
process.stdin.on('data', (chunk) => { data += chunk; });
process.stdin.on('end', () => {
  process.stdout.write(Buffer.from(data, 'utf8').toString('base64'));
});
`;

const SLOW_NODE = `setTimeout(() => {}, 30000);`;

// The minter spawns `encoderPath` directly with execFile (no shell), so the
// fixture must be a file the OS can exec. POSIX runs a shebang script. Windows
// cannot exec .mjs (spawn EFTYPE) or .bat without a shell, so tests that need
// a real spawn are POSIX-only; error paths stay portable.
const POSIX = process.platform !== 'win32';

function writeEncoderScript(t, nodeBody) {
  const directory = makeDir(t, 'castle-encoder-');
  const script = path.join(directory, 'encode-token.mjs');
  fs.writeFileSync(script, '#!' + process.execPath + '\n' + nodeBody, { mode: 0o755 });
  return script;
}

test('minter requires an encoder path', () => {
  assert.throws(() => createCastleMinter(), /encoderPath is required/);
});

test('minter returns a token produced by the encoder', { skip: !POSIX }, async (t) => {
  const encoder = writeEncoderScript(t, ECHO_NODE);
  const minter = createCastleMinter({
    encoderPath: encoder,
    tracePath: emptyTrace(t),
    budgetMs: 500,
    timeoutMs: 3000,
  });
  const result = await minter.mint({ cuid: 'fixture-cuid' });
  assert.equal(typeof result.token, 'string');
  assert.ok(result.token.length > 0);
  const frame = Buffer.from(result.token, 'base64');
  assert.equal(frame.subarray(0, 4).toString('latin1'), 'FCLT');
  assert.equal(minter.stats().mints, 1);
  assert.equal(result.cuid, 'fixture-cuid');
});

test('minter rejects concurrent mints while busy', { skip: !POSIX }, async (t) => {
  const encoder = writeEncoderScript(t, SLOW_NODE);
  const minter = createCastleMinter({
    encoderPath: encoder,
    tracePath: emptyTrace(t),
    budgetMs: 10000,
    timeoutMs: 200,
  });
  const first = minter.mint();
  await assert.rejects(minter.mint(), /busy/);
  await assert.rejects(first);
  assert.equal(minter.stats().failures, 2);
});

test('encoder subprocess is killed when it exceeds timeoutMs', { skip: !POSIX }, async (t) => {
  const encoder = writeEncoderScript(t, SLOW_NODE);
  const minter = createCastleMinter({
    encoderPath: encoder,
    tracePath: emptyTrace(t),
    budgetMs: 50,
    timeoutMs: 300,
  });
  const started = Date.now();
  await assert.rejects(minter.mint(), /encode-token/);
  assert.ok(Date.now() - started < 5000, 'encoder timeout should not wait for the 30s child');
  assert.equal(minter.stats().mints, 0);
  assert.equal(minter.stats().failures, 1);
});

test('encoder spawn failure surfaces a typed encode-token error', async (t) => {
  const missing = path.join(makeDir(t, 'castle-missing-'), 'no-such-encoder');
  const minter = createCastleMinter({
    encoderPath: missing,
    tracePath: emptyTrace(t),
    budgetMs: 2000,
    timeoutMs: 3000,
  });
  await assert.rejects(minter.mint({ cuid: 'fixture-cuid' }), /encode-token/);
  assert.equal(minter.stats().mints, 0);
  assert.equal(minter.stats().failures, 1);
});
