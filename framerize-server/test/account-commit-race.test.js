import assert from 'node:assert/strict';
import test from 'node:test';
import * as files from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, version as esbuildVersion } from 'esbuild';

const hookKey = 'framerize.test.accountCommitRename';
const digest = value => createHash('sha256').update(value).digest('hex');
const entry = fileURLToPath(new URL('../src/account-cli.ts', import.meta.url));
const compiled = await build({ entryPoints: [entry], bundle: true,
  platform: 'node', format: 'esm', write: false, metafile: true,
  plugins: [{ name: 'owned-rename-barrier', setup(builder) {
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
  sources[path] = digest(await files.readFile(path));
}
console.log(JSON.stringify({ evidence: 'source API with one synthetic rename barrier',
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

const add = id => ['add', id, '--project-id', `project_${id}`,
  '--session-env', 'SYNTHETIC_SESSION', '--json'];

test('concurrent account commits never lose an acknowledged addition', { timeout: 8000 }, async context => {
  const root = await files.mkdtemp(join(tmpdir(), 'frm-account-commit-race-'));
  const path = join(root, 'accounts.json');
  const entered = Promise.withResolvers();
  const release = Promise.withResolvers();
  const pending = [];
  let captured = false;
  let cleaned = false;
  const cleanup = async () => {
    if (cleaned) return;
    release.resolve();
    try { await bounded(Promise.all(pending), 'Account cleanup did not settle'); }
    finally {
      delete globalThis[Symbol.for(hookKey)];
      await files.rm(root, { recursive: true, force: true });
      await assert.rejects(files.access(root), { code: 'ENOENT' });
      cleaned = true;
    }
  };
  context.after(cleanup);
  globalThis[Symbol.for(hookKey)] = async (source, destination) => {
    if (destination === path && !captured) {
      captured = true; entered.resolve(); await release.promise;
    }
    return files.rename(source, destination);
  };
  await files.writeFile(path, JSON.stringify([{ id: 'original', projectId: 'project_original',
    session: { env: 'SYNTHETIC_SESSION' } }]), { mode: 0o600 });
  const run = id => {
    const output = [];
    const command = runAccountsCommand(add(id), {
      env: { FRAMERIZE_SERVER_HOME: root }, stdout: { write: text => output.push(text) },
    }).then(code => ({ id, code, output: output.join('') }),
      error => ({ id, error, output: output.join('') }));
    pending.push(command);
    return command;
  };
  const first = run('first');
  await bounded(entered.promise, 'First account rename was not reached');
  const second = run('second');
  let timer;
  const secondSettledBeforeRelease = await Promise.race([second.then(() => true),
    new Promise(resolve => { timer = setTimeout(() => resolve(false), 400); })]);
  clearTimeout(timer);
  const beforeRelease = JSON.parse(await files.readFile(path, 'utf8')).map(item => item.id).sort();
  release.resolve();
  const results = await bounded(Promise.all([first, second]), 'Account commands did not settle');
  const ids = JSON.parse(await files.readFile(path, 'utf8')).map(item => item.id).sort();
  const remaining = await files.readdir(root);
  await cleanup();
  const acknowledged = results.filter(result => result.code === 0 && !result.error);
  const expected = ['original', ...acknowledged.map(result => result.id)].sort();
  context.diagnostic(JSON.stringify({ secondSettledBeforeRelease, beforeRelease, ids, expected,
    results: results.map(result => ({ id: result.id, code: result.code,
      rejected: Boolean(result.error) })), privateStateRemoved: cleaned }));
  assert.ok(acknowledged.length > 0, 'At least one independent addition must succeed');
  for (const result of acknowledged) assert.equal(JSON.parse(result.output).id, result.id);
  assert.ok(remaining.every(name => !name.endsWith('.tmp')), 'Atomic staging files must be removed');
  assert.deepEqual(ids, expected, 'Every acknowledged account addition must remain in the file');
});
