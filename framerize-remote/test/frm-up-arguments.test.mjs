import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/frmCli.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { runFrm } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

for (const args of [[], ['up']]) for (const event of ['SIGINT', 'SIGTERM']) {
  test(`frm ${args.join(' ')} cancels its first-run question on ${event}`, { timeout: 2000 }, async t => {
    const home = await mkdtemp(join(tmpdir(), 'frm-up-question-'));
    const names = ['FRM_HOME', 'FRM_REMOTE'];
    const previous = names.map(name => process.env[name]);
    const listeners = ['SIGINT', 'SIGTERM'].map(name => process.listenerCount(name));
    t.after(async () => {
      names.forEach((name, i) => {
        if (previous[i] === undefined) delete process.env[name];
        else process.env[name] = previous[i];
      });
      await rm(home, { recursive: true, force: true });
    });
    process.env.FRM_HOME = home;
    delete process.env.FRM_REMOTE;
    const io = { stdout() { assert.fail('Cancelled setup must not print success'); }, stderr() {},
      ask(_question, signal) {
        assert.ok(signal instanceof AbortSignal);
        return new Promise((_resolve, reject) => {
          signal.addEventListener('abort', () => reject(signal.reason), { once: true });
          queueMicrotask(() => process.emit(event));
        });
      } };
    await assert.rejects(runFrm(args, io, { program: ['/synthetic/frm'] }), { code: 'aborted' });
    await assert.rejects(readFile(join(home, 'framerize-remote.json')), { code: 'ENOENT' });
    assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => process.listenerCount(name)), listeners);
  });
}

for (const args of [
  ['up', 'http://127.0.0.1:1', 'unexpected'],
  ['up', '--json', 'http://127.0.0.1:1', 'unexpected'],
]) test(`frm ${args.join(' ')} rejects extra arguments before reading a profile`, async t => {
  const previous = process.env.FRM_HOME;
  process.env.FRM_HOME = 'invalid-relative-home';
  t.after(() => {
    if (previous === undefined) delete process.env.FRM_HOME;
    else process.env.FRM_HOME = previous;
  });
  const io = { stdout() { assert.fail('Unexpected output'); },
    stderr() { assert.fail('Unexpected output'); },
    async ask() { assert.fail('Invalid arguments must not prompt'); } };
  await assert.rejects(runFrm(args, io, { program: ['/synthetic/frm'] }),
    { code: 'command_arguments' });
});
