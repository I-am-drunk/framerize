import assert from 'node:assert/strict';
import test from 'node:test';
import * as files from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, version as esbuildVersion } from 'esbuild';

const hookKey = 'framerize.test.accountRollbackRename';
const digest = value => createHash('sha256').update(value).digest('hex');
const root = fileURLToPath(new URL('../../', import.meta.url));
const compiled = await build({ absWorkingDir: root, preserveSymlinks: true,
  entryPoints: ['framerize-server/src/account-cli.ts'], bundle: true,
  platform: 'node', format: 'esm', write: false, metafile: true,
  plugins: [{ name: 'owned-settings-failure', setup(builder) {
    builder.onResolve({ filter: /^node:fs\/promises$/ }, args =>
      args.namespace === 'file' ? { path: 'owned-files', namespace: 'fixture' } : undefined);
    builder.onLoad({ filter: /.*/, namespace: 'fixture' }, () => ({
      contents: `export * from 'node:fs/promises';
        export const rename = (...args) => globalThis[Symbol.for('${hookKey}')](...args);`,
      loader: 'js',
    }));
  } }],
});
globalThis[Symbol.for(hookKey)] = files.rename;
const bundle = compiled.outputFiles[0].contents;
const { runAccountsCommand } = await import(
  `data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`);
const sources = {};
for (const path of Object.keys(compiled.metafile.inputs).filter(path => path.endsWith('.ts'))) {
  sources[path] = digest(await files.readFile(join(root, path)));
}
console.log(JSON.stringify({ evidence: 'source API with synthetic settings rename failure',
  runtime: process.version, esbuildVersion, sources, bundle: digest(bundle),
  test: digest(await files.readFile(new URL(import.meta.url))) }));

async function bounded(promise, label, milliseconds = 2000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error(label)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function optionalText(path) {
  return files.readFile(path, 'utf8').catch(error => {
    if (error.code === 'ENOENT') return undefined;
    throw error;
  });
}

const add = id => ['add', id, '--project-id', `project_${id}`,
  '--session-env', 'SYNTHETIC_SESSION', '--json'];

for (const existing of [true, false]) {
  test(`failed selection preserves a concurrent add with ${existing ? 'existing' : 'new'} accounts`,
    { timeout: 8000 }, async context => {
      const home = await files.mkdtemp(join(tmpdir(), 'frm-account-rollback-race-'));
      const path = join(home, 'accounts.json');
      const settingsPath = join(home, 'server.json');
      const entered = Promise.withResolvers();
      const release = Promise.withResolvers();
      const pending = [];
      let captured = false;
      let cleaned = false;
      const failure = Object.assign(new Error('Synthetic settings replacement denied'), { code: 'EACCES' });
      const cleanup = async () => {
        if (cleaned) return;
        release.resolve();
        try { await bounded(Promise.all(pending), 'Account cleanup did not settle'); }
        finally {
          globalThis[Symbol.for(hookKey)] = files.rename;
          await files.rm(home, { recursive: true, force: true });
          await assert.rejects(files.access(home), { code: 'ENOENT' });
          cleaned = true;
        }
      };
      context.after(cleanup);
      globalThis[Symbol.for(hookKey)] = async (source, destination) => {
        if (destination === settingsPath && !captured) {
          captured = true; entered.resolve(); await release.promise;
          throw failure;
        }
        return files.rename(source, destination);
      };
      const original = existing ? JSON.stringify([{ id: 'original', projectId: 'project_original',
        session: { env: 'SYNTHETIC_SESSION' } }]) : undefined;
      if (original !== undefined) await files.writeFile(path, original, { mode: 0o600 });
      const run = (id, select = false) => {
        const output = [];
        const command = runAccountsCommand([...add(id), ...(select ? ['--select'] : [])], {
          env: { FRAMERIZE_SERVER_HOME: home }, stdout: { write: text => output.push(text) },
        }).then(code => ({ id, code, output: output.join('') }),
          error => ({ id, error, output: output.join('') }));
        pending.push(command);
        return command;
      };
      const first = run('first', true);
      await bounded(entered.promise, 'The selection settings write was not reached');
      const second = run('second');
      let timer;
      const secondSettledBeforeRelease = await Promise.race([second.then(() => true),
        new Promise(resolve => { timer = setTimeout(() => resolve(false), 400); })]);
      clearTimeout(timer);
      const beforeRelease = JSON.parse((await optionalText(path)) ?? '[]').map(item => item.id).sort();
      release.resolve();
      const [firstResult, secondResult] = await bounded(Promise.all([first, second]),
        'Account commands did not settle after the settings failure');
      const after = await optionalText(path);
      const ids = JSON.parse(after ?? '[]').map(item => item.id).sort();
      const settings = await optionalText(settingsPath);
      const remaining = await files.readdir(home);
      await cleanup();
      context.diagnostic(JSON.stringify({ existing, secondSettledBeforeRelease, beforeRelease, ids,
        filePresent: after !== undefined, firstRejected: Boolean(firstResult.error),
        firstError: firstResult.error?.message, secondCode: secondResult.code,
        secondRejected: Boolean(secondResult.error), privateStateRemoved: cleaned }));
      assert.ok(firstResult.error, 'The denied settings write must not report success');
      assert.match(firstResult.error.message, /Synthetic settings replacement denied/);
      assert.equal(firstResult.output, '');
      assert.equal(settings, undefined);
      assert.ok(remaining.every(name => !name.endsWith('.tmp')), 'Staging files must be removed');
      if (secondResult.error || secondResult.code !== 0) {
        assert.equal(secondResult.output, '');
        assert.equal(after, original, 'With no acknowledged add, rollback restores the original bytes');
      } else {
        assert.equal(JSON.parse(secondResult.output).id, 'second');
        assert.deepEqual(ids.filter(id => id !== 'first'), existing ? ['original', 'second'] : ['second'],
          'A failed selection must not erase another command\'s acknowledged account');
      }
    });
}
