import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import * as files from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, version as esbuildVersion } from 'esbuild';

const key = 'framerize.test.accountTransaction';
const hooks = globalThis[Symbol.for(key)] = {};
const root = fileURLToPath(new URL('../../', import.meta.url));
const compiled = await build({ absWorkingDir: root, bundle: true, platform: 'node',
  format: 'esm', write: false, metafile: true, stdin: { resolveDir: root, contents:
    "export { runAccountsCommand } from './framerize-server/src/account-cli.ts';"
    + "export { withAccountLock } from './framerize-server/src/account-store.ts';" },
  plugins: [{ name: 'private-file-hooks', setup(builder) {
    builder.onResolve({ filter: /^node:fs\/promises$/ }, args => args.namespace === 'file'
      ? { path: 'files', namespace: 'fixture' } : undefined);
    builder.onLoad({ filter: /.*/, namespace: 'fixture' }, () => ({ loader: 'js', contents:
      "import * as files from 'node:fs/promises'; export * from 'node:fs/promises';"
      + ['rename', 'open', 'rm'].map(name => `export const ${name} = (...args) =>
        (globalThis[Symbol.for('${key}')]?.${name} ?? files.${name})(...args);`).join('\n') }));
  } }],
});
const bytes = compiled.outputFiles[0].contents;
const { runAccountsCommand, withAccountLock } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
const digest = data => createHash('sha256').update(data).digest('hex');
const sources = {};
for (const path of Object.keys(compiled.metafile.inputs).filter(path => path.endsWith('.ts'))) {
  sources[path] = digest(await files.readFile(join(root, path)));
}
console.log(JSON.stringify({ runtime: process.version, esbuildVersion, sources,
  test: digest(await files.readFile(new URL(import.meta.url))), bundle: digest(bytes) }));

async function bounded(promise, label) {
  const error = new Error(`Account transaction fixture timed out: ${label}`);
  let timer;
  try { return await Promise.race([promise, new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(error), 3000);
  })]); } finally { clearTimeout(timer); }
}
const add = id => ['add', id, '--project-id', `project_${id}`,
  '--session-env', 'SYNTHETIC_SESSION', '--json'];
const original = JSON.stringify([{ id: 'original', projectId: 'project_original',
  session: { env: 'SYNTHETIC_SESSION' } }]);

async function fixture(context) {
  const directory = await files.mkdtemp(join(tmpdir(), 'frm-account-transaction-'));
  const home = join(directory, 'home');
  await files.mkdir(home);
  const accounts = join(home, 'accounts.json');
  await files.writeFile(accounts, original, { mode: 0o600 });
  context.after(async () => {
    for (const name of Object.keys(hooks)) delete hooks[name];
    await files.rm(directory, { recursive: true, force: true });
    await assert.rejects(files.access(directory), { code: 'ENOENT' });
  });
  const output = [];
  const deps = { env: { FRAMERIZE_SERVER_HOME: home }, stdout: { write: text => output.push(text) } };
  return { directory, home, accounts, lock: join(home, '.accounts.json.lock'), output, deps };
}

async function assertReleased(state) {
  await assert.rejects(files.access(state.lock), { code: 'ENOENT' });
  assert.ok((await files.readdir(state.home)).every(name => !name.endsWith('.tmp')));
}

const holder = `
import { rename } from 'node:fs/promises';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
const [bundle, home] = process.argv.slice(2);
const { runAccountsCommand } = await import(pathToFileURL(bundle).href);
const release = Promise.withResolvers();
process.once('message', () => release.resolve());
globalThis[Symbol.for('${key}')] = { rename: async (source, destination) => {
  if (destination === join(home, 'accounts.json')) {
    process.send({ kind: 'held' });
    await release.promise;
  }
  return rename(source, destination);
} };
const output = [];
try {
  const code = await runAccountsCommand(${JSON.stringify(add('first'))}, {
    env: { FRAMERIZE_SERVER_HOME: home }, stdout: { write: text => output.push(text) },
  });
  await new Promise(resolve => process.send({ kind: 'result', code, output: output.join('') }, resolve));
} catch (error) {
  await new Promise(resolve => process.send({ kind: 'result', error: error.message }, resolve));
  process.exitCode = 1;
}
process.disconnect();
`;

test('separate account writers share a lock through directory aliases and can retry', { timeout: 10000 },
  async context => {
    const state = await fixture(context);
    const alias = join(state.directory, 'alias');
    await files.symlink(state.home, alias, 'junction');
    const modulePath = join(state.directory, 'accounts.mjs');
    const callerPath = join(state.directory, 'caller.mjs');
    await files.writeFile(modulePath, bytes);
    await files.writeFile(callerPath, holder);
    const child = spawn(process.execPath, [callerPath, modulePath, state.home], {
      cwd: state.directory, env: { HOME: state.directory, TMPDIR: state.directory },
      stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
    });
    const entered = Promise.withResolvers(), outcome = Promise.withResolvers();
    let stderr = '';
    child.stdout.resume();
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.on('message', message => {
      if (message.kind === 'held') entered.resolve();
      if (message.kind === 'result') outcome.resolve(message);
    });
    const closed = new Promise(resolve => {
      child.once('error', error => resolve({ error }));
      child.once('close', (code, signal) => resolve({ code, signal }));
    });
    try {
      await bounded(Promise.race([entered.promise, closed.then(value => {
        throw new Error(`Writer closed before its rename: ${JSON.stringify(value)} ${stderr}`);
      })]), 'first writer rename');
      assert.equal((await files.readFile(state.lock, 'utf8')).trim(), String(child.pid));
      await assert.rejects(runAccountsCommand(add('second'), {
        ...state.deps, env: { FRAMERIZE_SERVER_HOME: alias },
      }), /Another command is editing this accounts file/);
      assert.deepEqual(state.output, []);
      assert.equal(await files.readFile(state.accounts, 'utf8'), original);
      child.send('release');
      const first = await bounded(outcome.promise, 'first writer result');
      assert.equal(first.code, 0, first.error);
      assert.equal(JSON.parse(first.output).id, 'first');
      assert.deepEqual(await bounded(closed, 'first writer exit'), { code: 0, signal: null });
      assert.equal(stderr, '');
      await assertReleased(state);
      assert.equal(await runAccountsCommand(add('second'), state.deps), 0);
      assert.deepEqual(JSON.parse(await files.readFile(state.accounts, 'utf8')).map(a => a.id),
        ['original', 'first', 'second']);
      assert.equal(await runAccountsCommand(['remove', 'first', '--json'], state.deps), 0);
      assert.deepEqual(JSON.parse(await files.readFile(state.accounts, 'utf8')).map(a => a.id),
        ['original', 'second']);
      await assertReleased(state);
    } finally {
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
      await bounded(closed, 'owned writer cleanup');
    }
  });

for (const mode of ['settings failure', 'cancellation']) {
  test(`account transaction releases its lock after ${mode} and rollback`, { timeout: 8000 }, async context => {
    const state = await fixture(context);
    const controller = new AbortController();
    const failure = new Error(`Synthetic ${mode}`);
    let injected = false;
    hooks.rename = async (source, destination) => {
      if (!injected && destination === (mode === 'cancellation' ? state.accounts : join(state.home, 'server.json'))) {
        injected = true;
        if (mode === 'settings failure') throw failure;
        controller.abort(failure);
      }
      return files.rename(source, destination);
    };
    await assert.rejects(runAccountsCommand([...add('first'), '--select'], {
      ...state.deps, signal: controller.signal,
    }), error => error === failure);
    assert.ok(injected);
    assert.deepEqual(state.output, []);
    assert.equal(await files.readFile(state.accounts, 'utf8'), original);
    await assert.rejects(files.access(join(state.home, 'server.json')), { code: 'ENOENT' });
    await assertReleased(state);
    delete hooks.rename;
    assert.equal(await runAccountsCommand(add('retry'), state.deps), 0);
    assert.equal(await runAccountsCommand(['remove', 'retry', '--json'], state.deps), 0);
    assert.deepEqual(JSON.parse(await files.readFile(state.accounts, 'utf8')), JSON.parse(original));
    await assertReleased(state);
  });
}

test('lock cleanup reports both failures and preserves the operation error', async context => {
  const state = await fixture(context);
  const failure = new Error('Synthetic edit refusal');
  const closeFailure = new Error('Synthetic close report');
  const removeFailure = new Error('Synthetic remove report');
  let closes = 0, removes = 0;
  hooks.open = async (...args) => {
    const handle = await files.open(...args);
    if (args[0] !== state.lock) return handle;
    return { writeFile: (...values) => handle.writeFile(...values), close: async () => {
      closes++; await handle.close(); throw closeFailure;
    } };
  };
  hooks.rm = async (...args) => {
    await files.rm(...args);
    if (args[0] === state.lock) { removes++; throw removeFailure; }
  };
  await assert.rejects(withAccountLock(state.accounts, async () => { throw failure; }), error => {
    assert.equal(error.name, 'AccountCommandError');
    assert.match(error.message, /Could not release the accounts file lock/);
    assert.deepEqual(error.cause.errors, [failure, closeFailure, removeFailure]);
    return true;
  });
  assert.equal(closes, 1);
  assert.equal(removes, 1);
  delete hooks.open;
  delete hooks.rm;
  await assertReleased(state);
  assert.equal(await runAccountsCommand(add('retry'), state.deps), 0);
  await assertReleased(state);
});

test('a pre-aborted edit does not create a lock directory', async context => {
  const state = await fixture(context);
  const path = join(state.directory, 'absent', 'accounts.json');
  const reason = new Error('Synthetic cancellation before locking');
  await assert.rejects(withAccountLock(path, async () => {
    assert.fail('A cancelled operation must not start');
  }, AbortSignal.abort(reason)), error => error === reason);
  await assert.rejects(files.access(join(state.directory, 'absent')), { code: 'ENOENT' });
});
