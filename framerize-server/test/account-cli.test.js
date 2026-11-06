import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { run } from '../dist/cli.js';

const SESSION = 'synthetic-session-value-1234';
async function home(t) {
  const dir = await mkdtemp(join(tmpdir(), 'account-cli-test-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  return dir;
}
function cli(dir, extra = {}) {
  let out = '';
  const stdout = { write(text) { out += text; } };
  const process = { stdout, stderr: stdout, on() {}, off() {} };
  return { read: () => out, run: (argv, deps = {}) => run(argv, { process, env: { FRAMERIZE_SERVER_HOME: dir, ...extra },
    readStdin: async () => `${SESSION}\n`, ...deps }) };
}

test('add from stdin, list and show never print the session value', async t => {
  const dir = await home(t);
  const c = cli(dir);
  assert.equal(await c.run(['accounts', 'add', 'main', '--project-id', 'proj_1', '--session-stdin']), 0);
  const stored = JSON.parse(await readFile(join(dir, 'accounts.json'), 'utf8'));
  assert.deepEqual(stored, [{ id: 'main', projectId: 'proj_1', session: SESSION }]);
  if (process.platform !== 'win32') assert.equal((await stat(join(dir, 'accounts.json'))).mode & 0o077, 0);
  const json = cli(dir);
  await json.run(['accounts', 'list', '--json']);
  const listed = JSON.parse(json.read());
  await c.run(['accounts', 'show', 'main']);
  await c.run(['accounts']);
  assert.doesNotMatch(c.read() + json.read(), new RegExp(SESSION));
  assert.equal(listed.accounts[0].session, 'saved');
  assert.equal(listed.selected, null);
});

test('select persists the Framer choice so check runs without --config or exports', async t => {
  const dir = await home(t);
  const c = cli(dir);
  await c.run(['accounts', 'add', 'one', '--project-id', 'proj_1', '--session-env', 'ONE_SESSION']);
  await c.run(['accounts', 'add', 'two', '--project-id', 'proj_2', '--session-stdin']);
  assert.equal(await c.run(['accounts', 'use', 'two']), 0);
  const saved = JSON.parse(await readFile(join(dir, 'server.json'), 'utf8'));
  assert.equal(saved.config.backend, 'framer');
  assert.equal(saved.config.accountId, 'two');
  assert.equal(saved.config.accountsFile, join(dir, 'accounts.json'));
  let used;
  assert.equal(await c.run(['check'], { logger: { info() {} },
    createProvider(config) { used = config; return {}; },
    createServer() { return { listen() {}, close() {} }; } }), 0);
  assert.equal(used.accountId, 'two');
  assert.equal(used.backend, 'framer');
});

test('disable and remove clear a saved selection; enable keeps it clear', async t => {
  const dir = await home(t);
  const c = cli(dir);
  await c.run(['accounts', 'add', 'main', '--project-id', 'proj_1', '--session-stdin', '--select']);
  assert.equal(await c.run(['accounts', 'disable', 'main']), 0);
  let saved = JSON.parse(await readFile(join(dir, 'server.json'), 'utf8'));
  assert.equal(saved.config.accountId, undefined);
  assert.equal(JSON.parse(await readFile(join(dir, 'accounts.json'), 'utf8'))[0].enabled, false);
  await assert.rejects(c.run(['accounts', 'select', 'main']), /enabled account/);
  await c.run(['accounts', 'enable', 'main']);
  await c.run(['accounts', 'select', 'main']);
  assert.equal(await c.run(['accounts', 'remove', 'main', '--json']), 0);
  saved = JSON.parse(await readFile(join(dir, 'server.json'), 'utf8'));
  assert.equal(saved.config.accountId, undefined);
  assert.deepEqual(JSON.parse(await readFile(join(dir, 'accounts.json'), 'utf8')), []);
  await assert.rejects(c.run(['accounts', 'remove', 'main']), /Unknown account id/);
});

test('add --select restores the prior accounts bytes when the settings write fails', async t => {
  const dir = await home(t);
  const c = cli(dir);
  await c.run(['accounts', 'add', 'old', '--project-id', 'proj_1', '--session-env', 'OLD']);
  const before = await readFile(join(dir, 'accounts.json'), 'utf8');
  // readSettings accepts this file, but saving it fails validation after accounts.json was written.
  await writeFile(join(dir, 'server.json'), JSON.stringify({ version: 1,
    config: { serverTokenEnv: 'MISSING_SYNTHETIC_TOKEN' }, environment: {} }));
  await assert.rejects(c.run(['accounts', 'add', 'new', '--project-id', 'proj_2', '--session-stdin', '--select']));
  assert.equal(await readFile(join(dir, 'accounts.json'), 'utf8'), before);
  // A first add that created accounts.json removes it again.
  await rm(join(dir, 'accounts.json'));
  await assert.rejects(c.run(['accounts', 'add', 'new', '--project-id', 'proj_2', '--session-stdin', '--select']));
  await assert.rejects(stat(join(dir, 'accounts.json')), { code: 'ENOENT' });
});

test('oversized and FIFO account files fail without hanging or changing anything', async t => {
  const dir = await home(t);
  const c = cli(dir);
  const big = `[${' '.repeat(1024 * 1024)}]`;
  await writeFile(join(dir, 'accounts.json'), big);
  await assert.rejects(c.run(['accounts', 'list']), /Account metadata unavailable/);
  assert.equal(await readFile(join(dir, 'accounts.json'), 'utf8'), big);
  if (process.platform === 'win32') return;
  await rm(join(dir, 'accounts.json'));
  (await import('node:child_process')).execFileSync('mkfifo', [join(dir, 'accounts.json')]);
  await assert.rejects(c.run(['accounts', 'add', 'x', '--project-id', 'p', '--session-stdin']),
    /Account metadata unavailable/);
  assert.equal((await stat(join(dir, 'accounts.json'))).isFIFO(), true);
});

test('cancellation during session input writes nothing on the noninteractive path', async t => {
  const dir = await home(t);
  const events = new (await import('node:events')).EventEmitter();
  events.stdout = events.stderr = { write() {} };
  const pending = run(['accounts', 'add', 'main', '--project-id', 'proj_1', '--session-stdin', '--select'], {
    process: events, env: { FRAMERIZE_SERVER_HOME: dir },
    readStdin: () => new Promise(resolve => { events.emit('SIGINT'); setImmediate(() => resolve(SESSION)); }) });
  await assert.rejects(pending, /cancelled/);
  await assert.rejects(stat(join(dir, 'accounts.json')), { code: 'ENOENT' });
  await assert.rejects(stat(join(dir, 'server.json')), { code: 'ENOENT' });
  assert.equal(events.listenerCount('SIGINT'), 0);
});

const TOKEN = 'synthetic_server_token_5678';
for (const form of ['saved', 'environment']) {
  test(`configure --show omits a literal server token from ${form} settings`, async t => {
    const dir = await home(t);
    if (form === 'saved') {
      await writeFile(join(dir, 'server.json'), JSON.stringify({ version: 1,
        config: { serverToken: TOKEN, serverTokenEnv: '' }, environment: {} }));
    }
    const c = cli(dir, form === 'environment' ? { FRAMERIZE_SERVER_TOKEN: TOKEN } : {});
    await c.run(['configure', '--show']);
    const human = c.read();
    const json = cli(dir, form === 'environment' ? { FRAMERIZE_SERVER_TOKEN: TOKEN } : {});
    await json.run(['configure', '--show', '--json']);
    assert.doesNotMatch(human + json.read(), new RegExp(TOKEN));
    assert.equal(JSON.parse(json.read()).serverToken, 'set');
    assert.match(human, /serverToken: set/);
  });
}

test('configure http saves the upstream and credential name without printing its value', async t => {
  const dir = await home(t);
  const c = cli(dir, { UP_KEY: 'synthetic_upstream_key' });
  assert.equal(await c.run(['configure', '--backend', 'http', '--upstream', 'http://127.0.0.1:9/v1',
    '--authorization-env', 'UP_KEY', '--json']), 0);
  const saved = JSON.parse(await readFile(join(dir, 'server.json'), 'utf8'));
  assert.equal(saved.config.upstream, 'http://127.0.0.1:9/v1');
  assert.equal(saved.config.authorizationEnv, 'UP_KEY');
  assert.equal(saved.environment.UP_KEY, 'synthetic_upstream_key');
  assert.doesNotMatch(c.read(), /synthetic_upstream_key/);
  assert.equal(JSON.parse(c.read()).credential.env, 'UP_KEY');
});

test('invalid input and a session in argv leave the saved configuration unchanged', async t => {
  const dir = await home(t);
  const c = cli(dir);
  await c.run(['accounts', 'add', 'main', '--project-id', 'proj_1', '--session-stdin', '--select']);
  const before = await readFile(join(dir, 'server.json'), 'utf8');
  await assert.rejects(c.run(['configure', '--backend', 'framer', '--account-id', 'missing']), /Unknown account/);
  await assert.rejects(c.run(['configure', '--backend', 'other']), /framer or http/);
  await assert.rejects(c.run(['accounts', 'add', 'x', '--project-id', 'p', '--session', SESSION]), /Invalid option/);
  await assert.rejects(c.run(['accounts', 'add', 'x', '--project-id', 'p']), /--session-stdin/);
  assert.equal(await readFile(join(dir, 'server.json'), 'utf8'), before);
});

test('the terminal menu selects by number and quitting changes nothing', async t => {
  const dir = await home(t);
  const c = cli(dir);
  await c.run(['accounts', 'add', 'one', '--project-id', 'proj_1', '--session-stdin']);
  await c.run(['accounts', 'add', 'two', '--project-id', 'proj_2', '--session-stdin']);
  const answers = (list) => ({ ask: async () => list.shift() ?? '' });
  assert.equal(await c.run(['accounts'], answers(['quit'])), 0);
  await assert.rejects(stat(join(dir, 'server.json')), { code: 'ENOENT' });
  assert.equal(await c.run(['accounts'], answers(['select', '2'])), 0);
  assert.equal(JSON.parse(await readFile(join(dir, 'server.json'), 'utf8')).config.accountId, 'two');
  assert.equal(await c.run(['accounts'], answers(['remove', '1', 'n'])), 0);
  assert.equal(JSON.parse(await readFile(join(dir, 'accounts.json'), 'utf8')).length, 2);
  assert.doesNotMatch(c.read(), new RegExp(SESSION));
});

test('config prints the effective view and accounts login uses the injected hook', async t => {
  const dir = await home(t);
  const c = cli(dir);
  let calls = 0;
  await assert.rejects(c.run(['accounts', 'login', 'bad', '--project-id', 'bad/project'],
    { login: async () => { calls += 1; return SESSION; } }), /Invalid account project/);
  assert.equal(calls, 0);
  assert.equal(await c.run(['accounts', 'login', 'web', '--project-id', 'proj_w', '--json'],
    { login: async () => SESSION }), 0);
  assert.equal(JSON.parse(await readFile(join(dir, 'server.json'), 'utf8')).config.accountId, 'web');
  const view = cli(dir);
  assert.equal(await view.run(['config', '--json']), 0);
  assert.equal(JSON.parse(view.read()).config.accountId, 'web');
  assert.doesNotMatch(c.read() + view.read(), new RegExp(SESSION));
  await assert.rejects(cli(dir).run(['accounts', 'login', 'other', '--project-id', 'p']),
    error => error.name === 'BrowserLoginError' && error.code === 'unsupported');
});
