import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const { run } = await import(pathToFileURL(join(root, 'dist/index.js')).href);
const listeners = () => ['SIGINT', 'SIGTERM'].map(name => process.listeners(name));
const saved = '{"remote":"https://old.example","port":8788}\n';
async function fixture(t) {
  const directory = await mkdtemp(join(tmpdir(), 'frm-prompt-state-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const config = join(directory, 'profile.json');
  await writeFile(config, saved);
  const output = [];
  return { config, output, io: { stdout: text => output.push(text), stderr: text => output.push(text) } };
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  test(`setup preserves its profile when a late answer follows ${signal}`, async t => {
    const f = await fixture(t), before = listeners();
    await assert.rejects(run(['setup', '--json', '--config', f.config], { ...f.io,
      ask: async (_question, cancellation) => {
        assert.equal(cancellation.aborted, false);
        process.emit(signal);
        assert.equal(cancellation.aborted, true);
        return 'https://replacement.example';
      } }), { name: 'AbortError' });
    assert.equal(await readFile(f.config, 'utf8'), saved);
    assert.deepEqual(f.output, []);
    assert.deepEqual(listeners(), before);
  });
}

test('setup removes its signal handlers when the question fails', async t => {
  const f = await fixture(t), before = listeners(), failure = new Error('synthetic prompt failure');
  await assert.rejects(run(['setup', '--json', '--config', f.config], { ...f.io,
    ask: async () => { throw failure; } }), error => error === failure);
  assert.equal(await readFile(f.config, 'utf8'), saved);
  assert.deepEqual(listeners(), before);
});

test('setup still saves explicit options and removes its handlers after success', async t => {
  const f = await fixture(t), before = listeners();
  assert.equal(await run(['setup', '--json', '--config', f.config,
    '--remote', 'https://new.example', '--host', '127.0.0.1', '--port', '9898'], {
    ...f.io, ask: async () => assert.fail('Explicit remote must not prompt'),
  }), 0);
  const config = JSON.parse(await readFile(f.config, 'utf8'));
  assert.equal(config.remote, 'https://new.example');
  assert.equal(config.host, '127.0.0.1');
  assert.equal(config.port, 9898);
  assert.deepEqual(f.output, ['{"saved":true}\n']);
  assert.deepEqual(listeners(), before);
});
