import assert from 'node:assert/strict';
import test from 'node:test';
import { access, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const packageRoot = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const { runAccountsCommand } = await import(pathToFileURL(join(packageRoot, 'dist/account-cli.js')).href);
const add = id => ['add', id, '--project-id', `project_${id}`, '--session-stdin', '--json'];
const session = 'session=synthetic-pending-account';

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error('Synthetic account command did not settle')), 1500);
    })]);
  } finally { clearTimeout(timer); }
}

async function fixture(context) {
  const home = await mkdtemp(join(tmpdir(), 'framerize-account-edit-overlap-'));
  const pending = [];
  const input = deferred();
  context.after(async () => {
    input.resolve(session);
    try { await bounded(Promise.all(pending)); }
    finally {
      await rm(home, { recursive: true, force: true });
      await assert.rejects(access(home), { code: 'ENOENT' });
    }
  });
  const path = join(home, 'accounts.json');
  await writeFile(path, JSON.stringify([{ id: 'original', projectId: 'project_original',
    session: { env: 'SYNTHETIC_SESSION' } }]), { mode: 0o600 });
  const run = (argv, readStdin = async () => session) => {
    const output = [];
    const command = runAccountsCommand(argv, { env: { FRAMERIZE_SERVER_HOME: home },
      stdout: { write: text => output.push(text) }, readStdin }).then(
      code => ({ code, output: output.join('') }), error => ({ error, output: output.join('') }));
    pending.push(command);
    return command;
  };
  return { home, path, input, run };
}

for (const action of ['add', 'remove']) {
  test(`a completed ${action} survives an older add awaiting session input`, { timeout: 5000 }, async context => {
    const state = await fixture(context);
    const entered = deferred();
    const waiting = state.run(add('pending'), () => {
      entered.resolve();
      return state.input.promise;
    });
    await bounded(entered.promise);
    const completed = await bounded(state.run(action === 'add'
      ? add('completed') : ['remove', 'original', '--json']));
    const beforeResume = await readFile(state.path, 'utf8');
    state.input.resolve(session);
    const resumed = await bounded(waiting);
    const afterResume = await readFile(state.path, 'utf8');
    const ids = JSON.parse(afterResume).map(account => account.id).sort();
    const previousIds = JSON.parse(beforeResume).map(account => account.id).sort();
    context.diagnostic(JSON.stringify({ action, completedCode: completed.code,
      resumedCode: resumed.code, rejected: Boolean(resumed.error), previousIds, ids }));
    assert.ifError(completed.error);
    assert.equal(completed.code, 0);
    assert.deepEqual(previousIds, action === 'add' ? ['completed', 'original'] : []);
    assert.doesNotMatch(completed.output + resumed.output, /synthetic-pending-account/);
    assert.deepEqual(await readdir(state.home), ['accounts.json']);
    if (resumed.error || resumed.code !== 0) {
      assert.equal(afterResume, beforeResume, 'a rejected stale edit preserves the completed edit exactly');
      assert.equal(resumed.output, '', 'a rejected stale edit must not report a successful add');
    } else {
      assert.deepEqual(ids, action === 'add' ? ['completed', 'original', 'pending'] : ['pending']);
    }
  });
}

test('sequential account additions preserve all existing accounts', { timeout: 5000 }, async context => {
  const state = await fixture(context);
  for (const id of ['completed', 'pending']) {
    const result = await bounded(state.run(add(id)));
    assert.ifError(result.error);
    assert.equal(result.code, 0);
  }
  const accounts = JSON.parse(await readFile(state.path, 'utf8'));
  assert.deepEqual(accounts.map(account => account.id).sort(), ['completed', 'original', 'pending']);
  assert.deepEqual(await readdir(state.home), ['accounts.json']);
});
