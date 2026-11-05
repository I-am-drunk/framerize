import test from 'node:test';
import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import { EventEmitter } from 'node:events';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const packageRoot = process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT;
const { runFrm } = await import(packageRoot
  ? pathToFileURL(join(packageRoot, 'dist/index.js')).href : new URL('../dist/index.js', import.meta.url).href);

const counts = () => ['SIGINT', 'SIGTERM'].map(signal => process.listenerCount(signal));
const io = { stdout() {}, stderr(text) { assert.fail(text); } };

for (const signal of ['SIGINT', 'SIGTERM']) for (const code of [0, 23]) {
  test(`client launch releases preparation handlers before ${signal}, exit ${code}`, async () => {
    const root = await mkdtemp(join(tmpdir(), 'frm-client-transition-'));
    const before = counts();
    const originalEnv = process.env;
    const originalSpawn = childProcess.spawn;
    const executable = join(root, process.platform === 'win32' ? 'codex.exe' : 'codex');
    await writeFile(executable, 'synthetic executable, never run', { mode: 0o700 });
    let atLaunch, launchedFile, result;
    const forwarded = [];
    try {
      process.env = { HOME: root, USERPROFILE: root, PATH: root, PATHEXT: '.EXE',
        FRM_HOME: root, FRM_REMOTE: 'http://127.0.0.1:9' };
      childProcess.spawn = file => {
        atLaunch = counts();
        launchedFile = file;
        const child = new EventEmitter();
        child.kill = value => { forwarded.push(value); return true; };
        queueMicrotask(() => { process.emit(signal, signal); child.emit('close', code, null); });
        return child;
      };
      syncBuiltinESMExports();
      result = await runFrm(['client', 'codex'], io, { program: ['synthetic-frm'] });
    } finally {
      childProcess.spawn = originalSpawn;
      process.env = originalEnv;
      syncBuiltinESMExports();
      await rm(root, { recursive: true, force: true });
    }
    assert.equal(launchedFile, executable);
    assert.deepEqual(atLaunch, before, 'preparation handlers must be removed before spawn');
    assert.deepEqual(counts(), before);
    assert.equal(result, code, 'the client exit code must be preserved after its signal');
    if (signal === 'SIGTERM') assert.deepEqual(forwarded, ['SIGTERM']);
  });
}
