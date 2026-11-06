import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/command.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const bundleHash = createHash('sha256').update(compiled.outputFiles[0].contents).digest('hex');
const { runClient } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

async function observe(operation, timeout = 1500) {
  let timer;
  try {
    return await Promise.race([operation, new Promise(resolve => {
      timer = setTimeout(() => resolve({ state: 'pending' }), timeout);
    })]);
  } finally { clearTimeout(timer); }
}

for (const cancel of [true, false]) {
  test(`client ownership lookup ${cancel ? 'cancels before release' : 'permits an ordinary launch'}`,
    { timeout: 6000, skip: process.platform === 'win32' }, async context => {
      const root = await fs.mkdtemp(join(tmpdir(), 'client-owner-cancel-'));
      const repository = join(root, 'repository');
      const home = join(root, 'home');
      const bin = join(root, 'bin');
      const local = join(repository, '.claude', 'settings.local.json');
      const original = Buffer.from('{"env":{"SYNTHETIC":"unchanged"}}\n');
      const entered = deferred(), release = deferred();
      const controller = new AbortController();
      const reason = new Error('synthetic ownership cancellation');
      const captures = [], runs = [], reads = [], output = [], owners = [];
      let operation;
      let fetches = 0;
      context.after(async () => {
        release.resolve();
        controller.abort(reason);
        try {
          if (operation) assert.notEqual((await observe(operation)).state, 'pending');
        } finally { await fs.rm(root, { recursive: true, force: true }); }
        await assert.rejects(fs.stat(root), { code: 'ENOENT' });
        context.diagnostic('cleanup: owned private state removed; no child or socket created');
      });
      for (const path of [home, bin, join(repository, '.git'), join(repository, '.claude')]) {
        await fs.mkdir(path, { recursive: true, mode: 0o700 });
      }
      await fs.writeFile(local, original, { mode: 0o600 });
      const deps = {
        platform: process.platform, home, cwd: repository, stateDir: join(root, 'state'),
        env: { HOME: home, PATH: bin }, signal: controller.signal,
        target: { origin: 'http://127.0.0.1:8788' }, realPath: path => path,
        probe: path => path === join(bin, 'claude'),
        io: { stdout: text => output.push(text), stderr: text => output.push(text) },
        read: async path => { reads.push(path); return path === local ? original.toString() : null; },
        fetch: async () => { fetches++; throw new Error('Unexpected release fetch'); },
        fileOwner: async path => {
          owners.push(path);
          const stat = await fs.lstat(path);
          if (cancel && path === repository) { entered.resolve(); await release.promise; }
          return stat.uid;
        },
        runner: {
          async capture(command) {
            captures.push(command);
            assert.equal(command.command, 'git');
            assert.deepEqual(command.args, ['-C', repository, 'worktree', 'list', '--porcelain', '-z']);
            return `worktree ${repository}\0HEAD synthetic\0\0`;
          },
          async run(command) { runs.push(command); return 0; },
        },
      };
      operation = runClient(['claude'], deps).then(
        code => ({ state: 'resolved', code }), error => ({ state: 'rejected', error }));
      let beforeRelease;
      if (cancel) {
        assert.equal((await observe(entered.promise.then(() => ({ state: 'entered' })))).state, 'entered');
        controller.abort(reason);
        beforeRelease = await observe(operation, 500);
        release.resolve();
      }
      const result = await observe(operation);
      assert.equal(result.state, cancel ? 'rejected' : 'resolved');
      if (cancel) assert.equal(result.error, reason);
      else assert.equal(result.code, 0);
      assert.deepEqual(await fs.readFile(local), original);
      assert.equal(captures.length, 1);
      assert.equal(owners.length, 3);
      assert.equal(fetches, 0);
      assert.equal(output.length, 0);
      assert.equal(runs.length, cancel ? 0 : 1);
      if (cancel) assert.equal(reads.length, 0);
      else {
        assert.equal(runs[0].command, join(bin, 'claude'));
        assert.ok(reads.includes(local));
      }
      context.diagnostic(`source bundle SHA256 ${bundleHash}`);
      context.diagnostic(JSON.stringify({ cancel, beforeRelease: beforeRelease?.state,
        eventual: result.state, launches: runs.length, fetches, unchangedState: true }));
      if (cancel) assert.equal(beforeRelease.state, 'rejected',
        'Client cancellation must settle while local ownership lookup is still pending');
    });
}
