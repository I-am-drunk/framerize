import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { spawn } from 'node:child_process';
import { mkdtemp, readdir, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { BrowserLoginError, FRAMER_LOGIN_URL, findBrowser, loginWithBrowser, sessionFromCookies,
  supportsBrowserLogin } from '../dist/browser-login.js';

const fakeBrowser = fileURLToPath(new URL('../fixtures/fake-browser.mjs', import.meta.url));
const session = 'session=synthetic-browser-session';
const expiresAt = () => new Date(Date.now() + 3_600_000).toISOString();
const gone = pid => { try { process.kill(pid, 0); return false; } catch (error) { return error.code === 'ESRCH'; } };

// Answers /auth/ai/access-token with the next queued status, then 200.
async function framer(context, statuses = []) {
  const cookies = [];
  const server = createServer((request, response) => {
    cookies.push(request.headers.cookie);
    const status = statuses.shift() ?? 200;
    response.writeHead(status, { 'content-type': 'application/json' });
    response.end(status === 200 ? JSON.stringify({ accessToken: 'synthetic-token', expiresAt: expiresAt() }) : '{}');
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  context.after(() => new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }));
  return { origin: `http://127.0.0.1:${server.address().port}`, cookies };
}

// A private temporary directory, a state file and a spawn wrapper that starts the fake browser.
async function harness(context, mode = 'ok') {
  const root = await mkdtemp(join(tmpdir(), 'framerize-browser-login-test-'));
  const children = [];
  context.after(async () => {
    for (const child of children) {
      if (child.pid) { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }
      if (child.exitCode === null && child.signalCode === null) {
        child.kill('SIGKILL');
        let timer;
        try { await Promise.race([once(child, 'exit'), new Promise((resolve, reject) => {
          timer = setTimeout(() => reject(new Error('Owned fake browser did not exit')), 2000);
        })]); } finally { clearTimeout(timer); }
      }
    }
    await rm(root, { recursive: true, force: true });
  });
  const state = join(root, 'state.json');
  const spawned = [];
  const fakeSpawn = (command, args, options) => {
    spawned.push({ command, args, options });
    const child = spawn(process.execPath, [fakeBrowser, mode, ...args],
      { ...options, env: { ...process.env, FAKE_BROWSER_STATE: state } });
    children.push(child);
    return child;
  };
  const notices = [];
  const options = {
    interactive: true, platform: 'linux', env: { DISPLAY: ':99' }, browser: process.execPath,
    spawn: fakeSpawn, temporaryDirectory: root, notice: text => notices.push(text),
    confirm: async () => {},
  };
  const browserState = async () => JSON.parse(await readFile(state, 'utf8'));
  const profiles = async () => (await readdir(root)).filter(name => name.startsWith('framerize-login-'));
  return { root, spawned, notices, options, browserState, profiles };
}

// Resolves on the first `times` calls, then waits until the login stops it.
const confirmTimes = (times = 1) => {
  let calls = 0;
  const confirm = signal => ++calls <= times ? Promise.resolve()
    : new Promise((resolve, reject) => signal.addEventListener('abort', () => reject(signal.reason), { once: true }));
  return { confirm, calls: () => calls };
};
async function assertCleaned(h) {
  const { pid } = await h.browserState();
  assert.ok(gone(pid), 'the owned browser process stopped');
  assert.deepEqual(await h.profiles(), [], 'the temporary profile was removed');
}
async function assertFails(promise, code) {
  const error = await promise.then(() => assert.fail('login succeeded'), error => error);
  assert.ok(error instanceof BrowserLoginError);
  assert.equal(error.code, code);
  assert.doesNotMatch(`${error.message} ${error.stack}`, /synthetic-(browser-session|decoy|error-text)/);
  return error;
}

test('a verified sign-in returns only the framer.com session and cleans up', async context => {
  const remote = await framer(context);
  const h = await harness(context);
  const result = await loginWithBrowser({ ...h.options, framerOrigin: remote.origin });
  assert.equal(result, session);
  assert.deepEqual(remote.cookies, [session]);
  const [{ command, args, options }] = h.spawned;
  assert.equal(command, process.execPath);
  assert.equal(args.at(-1), FRAMER_LOGIN_URL);
  assert.ok(args.includes('--remote-debugging-pipe'));
  assert.ok(!args.some(arg => /headless|remote-debugging-port|user-data-dir=.*google-chrome/.test(arg)));
  assert.deepEqual(options.stdio, ['ignore', 'ignore', 'ignore', 'pipe', 'pipe']);
  assert.match(args.find(arg => arg.startsWith('--user-data-dir=')), /framerize-login-/);
  assert.equal(h.notices.length, 1);
  assert.doesNotMatch(h.notices.join('\n'), /synthetic/);
  await assertCleaned(h);
});

test('only the session cookie on exactly framer.com is kept', () => {
  const cookie = (name, domain, value = 'synthetic-value') => ({ name, value, domain });
  assert.equal(sessionFromCookies({ cookies: [cookie('session', 'framer.com')] }), 'session=synthetic-value');
  for (const domain of ['evil.test', '.framer.com.evil.test', '.notframer.com', 'eu.framer.com', '.FRAMER.COM']) {
    assert.equal(sessionFromCookies({ cookies: [cookie('session', domain)] }), undefined, domain);
  }
  assert.equal(sessionFromCookies({ cookies: [cookie('__cf_bm', '.framer.com')] }), undefined);
  for (const value of ['', 'synthetic;x', 'synthetic value', 'synthetic"', 'x'.repeat(16_001), 7]) {
    assert.equal(sessionFromCookies({ cookies: [cookie('session', '.framer.com', value)] }), undefined);
  }
  for (const result of [undefined, null, [], {}, { cookies: 'synthetic' }]) {
    assert.throws(() => sessionFromCookies(result), error => error.code === 'protocol');
  }
});

test('a declined sign-in prompts again, and a refused session fails with a fixed code', async context => {
  const remote = await framer(context, [401]);
  const h = await harness(context);
  const prompt = confirmTimes(2);
  assert.equal(await loginWithBrowser({ ...h.options, confirm: prompt.confirm, framerOrigin: remote.origin }), session);
  assert.equal(prompt.calls(), 2);
  assert.equal(h.notices.length, 2);
  await assertCleaned(h);
  const refused = await framer(context, [403]);
  const second = await harness(context);
  await assertFails(loginWithBrowser({ ...second.options, framerOrigin: refused.origin }), 'auth_refused');
  await assertCleaned(second);
});

test('decoy cookies alone never count as a sign-in', async context => {
  const remote = await framer(context);
  const h = await harness(context, 'decoys');
  const prompt = confirmTimes(1);
  await assertFails(loginWithBrowser({ ...h.options, confirm: prompt.confirm, framerOrigin: remote.origin,
    timeoutMs: 1_500 }), 'timeout');
  assert.deepEqual(remote.cookies, [], 'no decoy reached Framer');
  assert.equal(h.notices.length, 2);
  await assertCleaned(h);
});

test('timeout, cancellation and a closed window stop the browser and remove the profile', async context => {
  const timedOut = await harness(context);
  await assertFails(loginWithBrowser({ ...timedOut.options, confirm: confirmTimes(0).confirm, timeoutMs: 200 }),
    'timeout');
  await assertCleaned(timedOut);
  const cancelled = await harness(context);
  const controller = new AbortController();
  setTimeout(() => controller.abort(), 200);
  const error = await assertFails(loginWithBrowser({ ...cancelled.options, confirm: confirmTimes(0).confirm,
    signal: controller.signal }), 'cancelled');
  assert.equal(error.exitCode, 130);
  await assertCleaned(cancelled);
  const closedInput = await harness(context);
  await assertFails(loginWithBrowser({ ...closedInput.options, confirm: async () => { throw new Error('closed'); } }),
    'cancelled');
  await assertCleaned(closedInput);
  const exited = await harness(context, 'exit');
  await assertFails(loginWithBrowser({ ...exited.options, confirm: confirmTimes(0).confirm }), 'browser_closed');
  await assertCleaned(exited);
});

test('malformed, oversized, unexpected and error replies fail as protocol errors', async context => {
  for (const mode of ['malformed', 'oversized', 'unknown-id', 'error']) {
    const h = await harness(context, mode);
    await assertFails(loginWithBrowser({ ...h.options, timeoutMs: 5_000 }), 'protocol');
    assert.doesNotMatch(h.notices.join('\n'), /synthetic|not-json/);
    await assertCleaned(h);
  }
});

test('an abort during browser lookup is not missed and launches nothing', async context => {
  const h = await harness(context);
  const controller = new AbortController();
  const login = loginWithBrowser({ ...h.options, signal: controller.signal });
  controller.abort();
  await assertFails(login, 'cancelled');
  const early = new AbortController();
  early.abort();
  await assertFails(loginWithBrowser({ ...h.options, signal: early.signal }), 'cancelled');
  assert.equal(h.spawned.length, 0);
  assert.deepEqual(await h.profiles(), []);
});

test('a helper that outlives the browser is killed by process group',
  { skip: process.platform === 'win32' && 'POSIX process groups are unavailable' }, async context => {
  const h = await harness(context, 'orphan-exit');
  await assertFails(loginWithBrowser({ ...h.options, confirm: confirmTimes(0).confirm }), 'browser_closed');
  const { helper } = await h.browserState();
  context.after(() => { try { process.kill(helper, 'SIGKILL'); } catch {} });
  assert.ok(gone(helper), 'the surviving helper stopped');
  await assertCleaned(h);
  const running = await harness(context, 'orphan');
  await assertFails(loginWithBrowser({ ...running.options, confirm: confirmTimes(0).confirm, timeoutMs: 300 }),
    'timeout');
  const state = await running.browserState();
  context.after(() => { try { process.kill(state.helper, 'SIGKILL'); } catch {} });
  assert.ok(gone(state.helper));
  await assertCleaned(running);
});

test('a browser that cannot be stopped is reported within a bounded wait',
  { skip: process.platform === 'win32' && 'POSIX process groups are unavailable' }, async context => {
  const h = await harness(context, 'stubborn');
  const refuse = (pid, signal) => { if (signal !== 0) throw new Error('synthetic-refused'); process.kill(pid, 0); };
  const started = Date.now();
  await assertFails(loginWithBrowser({ ...h.options, confirm: confirmTimes(0).confirm, timeoutMs: 200,
    kill: refuse }), 'cleanup_failed');
  assert.ok(Date.now() - started < 8_000, 'cleanup gave up instead of waiting forever');
  const { pid } = await h.browserState();
  assert.ok(!gone(pid), 'the refused browser is still running, so success was not claimed');
  process.kill(-pid, 'SIGKILL');
  for (let tries = 0; !gone(pid) && tries < 100; tries += 1) await new Promise(r => setTimeout(r, 20));
  assert.ok(gone(pid));
});

test('no terminal, no desktop, Windows or no browser exits 2 with the session-entry path', async context => {
  const h = await harness(context);
  const cases = [[{ interactive: false }, 'unsupported'], [{ env: {} }, 'unsupported'],
    [{ platform: 'win32' }, 'unsupported'], [{ browser: join(h.root, 'missing-browser') }, 'browser_missing']];
  for (const [change, code] of cases) {
    const error = await assertFails(loginWithBrowser({ ...h.options, ...change }), code);
    assert.equal(error.exitCode, 2);
    assert.match(error.message, /--session-stdin/);
  }
  assert.equal(supportsBrowserLogin({ interactive: true, platform: 'linux', env: { WAYLAND_DISPLAY: 'wayland-0' } }), true);
  assert.equal(await findBrowser({ env: { PATH: h.root }, platform: 'linux' }), undefined);
  assert.equal(h.spawned.length, 0);
  assert.deepEqual(await h.profiles(), []);
});
