const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const test = require('node:test');
const root = path.join(__dirname, '../framerize_host/browser');
const source = fs.readFileSync(path.join(root, 'supplied_start.js'), 'utf8')
  .replace('__READ_RESPONSE__', fs.readFileSync(path.join(root, 'response_reader.js'), 'utf8'));

function fixture(fetch) {
  const timers = new Map();
  const realm = vm.createContext({fetch, AbortController, TextDecoder,
    location: {origin: 'https://framer.com'}, Date: {now: () => 1000},
    localStorage: {getItem: () => 'actual-device'}, window: {},
    setTimeout: fn => { timers.set(1, fn); return 1; }, clearTimeout: id => timers.delete(id)});
  const start = vm.runInContext('(' + source + ')', realm);
  const request = {wire: '{"castleToken":"fresh-native-output"}', path: '/ai/v3/responses/',
    bearer: 'fixture-bearer', device: 'actual-device', expires_unix_ms: 2000,
    timeout_ms: 3000, response_limit: 1024};
  return {realm, start, request, timers};
}

test('sends the exact supplied token once with the browser fetch contract', async () => {
  const calls = [];
  const f = fixture(async (...args) => {
    calls.push(args);
    return new Response('ok', {status: 200, headers: {'content-type': 'text/plain'}});
  });
  await f.start([f.request, 'job']);
  await f.realm.window.__aiJobs.job.settled;
  assert.equal(calls.length, 1);
  assert.equal(calls[0][0], 'https://api.framer.com/ai/v3/responses/');
  const options = calls[0][1];
  assert.equal(options.body, f.request.wire);
  assert.equal(options.credentials, 'include');
  assert.equal(options.redirect, 'error');
  assert.equal(options.headers.authorization, 'Bearer fixture-bearer');
  assert.equal(f.realm.window.__aiJobs.job.contentType, 'text/plain');
  assert.equal(f.realm.window.__aiJobs.job.value.status, 200);
});

test('wrong origin, expired token and changed device refuse before fetch', async () => {
  for (const change of [f => f.realm.location.origin = 'https://other.invalid',
    f => f.request.expires_unix_ms = 1000, f => f.request.device = 'other-device']) {
    let calls = 0;
    const f = fixture(async () => { calls++; return new Response('unexpected'); });
    change(f);
    await assert.rejects(f.start([f.request, 'job']));
    assert.equal(calls, 0);
  }
});

test('actual reader preserves an inclusive UTF-8 byte limit', async () => {
  for (const limit of [4, 3]) {
    const f = fixture(async () => new Response('😀'));
    f.request.response_limit = limit;
    await f.start([f.request, 'job']);
    const job = f.realm.window.__aiJobs.job;
    await job.settled;
    assert.equal(job.value.status, limit === 4 ? 200 : -1);
  }
});

test('deadline abort joins the failed fetch without retry or private error text', async () => {
  let calls = 0;
  const f = fixture(async (_, options) => {
    calls++;
    return new Promise((resolve, reject) => options.signal.addEventListener('abort',
      () => reject(new Error('private-upstream-data'))));
  });
  await f.start([f.request, 'job']);
  const job = f.realm.window.__aiJobs.job;
  f.timers.get(1)();
  await job.settled;
  assert.equal(job.done, true);
  assert.equal(job.value.status, -1);
  assert.equal(calls, 1);
  assert.equal(JSON.stringify(job.value).includes('private-upstream-data'), false);
});
