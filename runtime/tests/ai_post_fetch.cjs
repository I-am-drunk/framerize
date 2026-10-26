'use strict';
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const {Script, createContext} = require('node:vm');
const {test} = require('node:test');
const scripts = JSON.parse(readFileSync(0, 'utf8'));

function fixture(fetcher) {
  const calls = [], timers = new Map();
  const realm = createContext({AbortController, TextDecoder,
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
    cancel: () => cancel('job'), expire: () => {
      const callback = timers.get(1);
      timers.delete(1);
      callback();
    }};
}
const flush = () => new Promise(resolve => setImmediate(resolve));
const plain = value => JSON.parse(JSON.stringify(value));
const streamed = (status, body) => ({status, body,
  headers: new Headers({'x-framer-ai-usage': 'usage', 'content-length': '1'}),
  text: () => new Response(body).text(),
});
const response = (status, text) => streamed(status,
  new ReadableStream({async start(controller) {
    try {
      controller.enqueue(new TextEncoder().encode(await text()));
      controller.close();
    } catch (error) { controller.error(error); }
  }}));
function chunked(status, chunks, cancel = () => {}) {
  let index = 0;
  return streamed(status, new ReadableStream({
    pull(controller) {
      if (index === chunks.length) controller.close();
      else controller.enqueue(chunks[index++]);
    }, cancel,
  }, {highWaterMark: 0}));
}
function drained(f) {
  assert.equal(f.calls.length, 1);
  assert.equal(f.timers.size, 0);
  assert.equal(f.realm.__aiJobs.job, undefined);
}

test('the exact byte limit is accepted despite an unhelpful length header', async () => {
  const part = new Uint8Array(32768).fill(97);
  const r = chunked(200, Array(scripts.AI_RESPONSE_LIMIT / part.length).fill(part));
  const f = fixture(() => r);
  await f.run();
  await flush();
  const result = f.poll();
  assert.equal(result.status, 200);
  assert.equal(result.body.length, scripts.AI_RESPONSE_LIMIT);
  assert.ok(result.body === 'a'.repeat(scripts.AI_RESPONSE_LIMIT), 'the full body is preserved');
  assert.equal(r.body.locked, false);
  assert.equal(f.calls[0][1].signal.aborted, false);
  drained(f);
});

test('split UTF-8, BOM and incomplete bytes match Response.text decoding', async () => {
  const bytes = Uint8Array.from([239, 187, 191, 97, 240, 159, 140, 141, 195, 40, 226]);
  const expected = await new Response(bytes).text();
  const f = fixture(() => chunked(200, Array.from(bytes, byte => Uint8Array.of(byte))));
  await f.run();
  await flush();
  assert.equal(f.poll().body, expected);
  drained(f);
});

test('oversized bodies abort without returning partial successful output', async () => {
  for (const status of [200, 403]) {
    for (const lengths of [[scripts.AI_RESPONSE_LIMIT, 1], [scripts.AI_RESPONSE_LIMIT + 1]]) {
      let cancellations = 0;
      const r = chunked(status, lengths.map(length => new Uint8Array(length)),
        () => { cancellations += 1; });
      const f = fixture(() => r);
      await f.run();
      await flush();
      const result = f.poll();
      assert.equal(result.status, -1, 'an oversized response must not succeed');
      assert.deepEqual(plain(result), {status: -1, usage: null,
        body: 'Error: response body limit exceeded', failure_phase: 'body', head_status: status});
      assert.equal(f.calls[0][1].signal.aborted, true);
      assert.equal(cancellations, 1);
      assert.equal(r.body.locked, false);
      drained(f);
    }
  }
});

test('an oversize failure is not published before reader cancellation settles', async () => {
  let release, cancelling = false;
  const cleanup = new Promise(resolve => { release = resolve; });
  const r = chunked(200, [new Uint8Array(scripts.AI_RESPONSE_LIMIT + 1)],
    () => { cancelling = true; return cleanup; });
  const f = fixture(() => r);
  await f.run();
  await flush();
  assert.equal(cancelling, true);
  assert.equal(f.poll(), null);
  assert.equal(r.body.locked, true);
  release();
  await flush();
  assert.equal(f.poll().failure_phase, 'body');
  assert.equal(r.body.locked, false);
  drained(f);
});

test('a response without a body retains the normal empty result', async () => {
  const f = fixture(() => streamed(204, null));
  await f.run();
  await flush();
  assert.deepEqual(plain(f.poll()), {status: 204, usage: 'usage', body: ''});
  drained(f);
});

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

test('the abandoned-job timer keeps unfinished cleanup joinable', async () => {
  let reject;
  const body = new Promise((_, fail) => { reject = fail; });
  const f = fixture(() => response(200, () => body));
  await f.run();
  await flush();
  f.expire();
  assert.equal(f.calls[0][1].signal.aborted, true);
  assert.ok(f.realm.__aiJobs.job);
  let joined = false;
  const cancel = f.cancel().then(value => { joined = true; return value; });
  await flush();
  assert.equal(joined, false);
  reject(new Error('synthetic-abandoned-body'));
  assert.equal(await cancel, true);
  drained(f);
});

test('cancellation prevents late body failure from recreating a job', {timeout: 1000}, async () => {
  let reject;
  const body = new Promise((_, fail) => { reject = fail; });
  const f = fixture(() => response(200, () => body));
  await f.run();
  await flush();
  let settled = false;
  const cancelling = f.cancel().then(value => { settled = true; return value; });
  assert.equal(f.calls[0][1].signal.aborted, true);
  await flush();
  assert.equal(settled, false, 'cancellation must retain the unsettled body');
  reject(new Error('synthetic-cancelled-body'));
  assert.equal(await cancelling, true);
  await flush();
  assert.equal(f.poll(), null);
  drained(f);
});
