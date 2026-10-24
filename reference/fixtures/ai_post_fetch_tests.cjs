'use strict';
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const {Script, createContext} = require('node:vm');
const {test} = require('node:test');
const scripts = JSON.parse(readFileSync(0, 'utf8'));

function fixture(fetcher) {
  const calls = [], timers = new Map();
  const realm = createContext({AbortController,
    setTimeout: (callback, delay) => {
      assert.equal(delay, 900000);
      timers.set(1, callback);
      return 1;
    },
    clearTimeout: id => timers.delete(id),
    fetch: async (...args) => { calls.push(args); return fetcher(...args); },
  });
  realm.window = realm;
  const start = new Script(scripts.AI_START_JS).runInContext(realm);
  const poll = new Script(scripts.AI_POLL_JS).runInContext(realm);
  const cancel = new Script(scripts.AI_CANCEL_JS).runInContext(realm);
  const run = () => start(['/ai/v3/responses/', {request: {stream: false}},
    'synthetic-token', 'synthetic-bearer', 'job', null]);
  return {calls, timers, realm, run, poll: () => poll('job'),
    cancel: () => cancel('job')};
}
const flush = () => new Promise(resolve => setImmediate(resolve));
const plain = value => JSON.parse(JSON.stringify(value));
const response = (status, text) => ({status, headers: {get: () => 'usage'}, text});
function drained(f) {
  assert.equal(f.calls.length, 1);
  assert.equal(f.timers.size, 0);
  assert.equal(f.realm.__aiJobs.job, undefined);
}

test('fetch rejection has no observed response head', {timeout: 1000}, async () => {
  const f = fixture(() => { throw new Error('synthetic-network-failure'); });
  await f.run();
  await flush();
  assert.deepEqual(plain(f.poll()), {status: -1, usage: null,
    body: 'Error: synthetic-network-failure', failure_phase: 'fetch', head_status: null});
  drained(f);
});

test('body rejection preserves 200 and 403 heads without changing status', {timeout: 1000}, async () => {
  for (const status of [200, 403]) {
    const f = fixture(() => response(status, async () => {
      throw new Error('synthetic-body-failure');
    }));
    await f.run();
    await flush();
    assert.deepEqual(plain(f.poll()), {status: -1, usage: null,
      body: 'Error: synthetic-body-failure', failure_phase: 'body', head_status: status});
    drained(f);
  }
});

test('completed responses keep their existing shape and request', {timeout: 1000}, async () => {
  const f = fixture(() => response(200, async () => 'synthetic-output'));
  await f.run();
  await flush();
  assert.deepEqual(plain(f.poll()), {status: 200, usage: 'usage', body: 'synthetic-output'});
  const [url, options] = f.calls[0];
  assert.equal(url, 'https://api.framer.com/ai/v3/responses/');
  assert.equal(options.credentials, 'include');
  assert.equal(options.headers.authorization, 'Bearer synthetic-bearer');
  assert.deepEqual(JSON.parse(options.body), {request: {stream: false}, castleToken: 'synthetic-token'});
  drained(f);
});

test('headers alone leave polling pending until the body settles', {timeout: 1000}, async () => {
  let reject;
  const body = new Promise((_, fail) => { reject = fail; });
  const f = fixture(() => response(403, () => body));
  await f.run();
  await flush();
  assert.equal(f.poll(), null);
  assert.equal(f.timers.size, 1);
  reject(new Error('synthetic-body-failure'));
  await flush();
  assert.equal(f.poll().head_status, 403);
  drained(f);
});

test('cancellation prevents late body failure from recreating a job', {timeout: 1000}, async () => {
  let reject;
  const body = new Promise((_, fail) => { reject = fail; });
  const f = fixture(() => response(200, () => body));
  await f.run();
  await flush();
  assert.equal(f.cancel(), true);
  assert.equal(f.calls[0][1].signal.aborted, true);
  reject(new Error('synthetic-cancelled-body'));
  await flush();
  assert.equal(f.poll(), null);
  drained(f);
});
