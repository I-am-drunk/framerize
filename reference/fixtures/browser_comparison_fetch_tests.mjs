// Inert source fixture. UNRUN by its source-only author. No SDK or network.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { Script } from 'node:vm';
import { test } from 'node:test';

const source = readFileSync(new URL('../../scripts/contextual_authorization.py', import.meta.url), 'utf8');
const marker = 'COMPARISON_FETCH = """';
const start = source.indexOf(marker);
assert.ok(start >= 0);
assert.equal(source.indexOf(marker, start + marker.length), -1);
const end = source.indexOf('"""', start + marker.length);
assert.ok(end > start);
const literal = source.slice(start + marker.length, end);
// The current Python literal has no escapes. Refuse future escaped source
// instead of silently testing a different string from Python's runtime value.
assert.equal(literal.includes('\\'), false);
const script = new Script('(' + literal + ')');
const utf8 = new TextEncoder();
const deferred = () => {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
};

class BrowserFixture {
  constructor() {
    this.page = { href: 'https://framer.com/projects/fixture-project' };
    this.device = 'fixture-device';
    this.cookie = '__cuid=fixture-device';
    this.chunks = [utf8.encode('{"id":"fixture-result"}')];
    this.calls = [];
    this.reads = this.cancelled = this.released = this.cleared = 0;
    this.status = 200;
    this.contentType = 'application/json';
    this.time = 0;
    this.readEntered = deferred();
    this.input = { page_url: this.page.href, device: this.device,
      path: '/ai/v3/responses/', timeout_ms: 30000, byte_limit: 65536,
      body: JSON.stringify({ projectId: 'fixture-project', sessionId: 'fixture-session',
        turnId: 'baseline-turn', request: { model: 'fixture-model', stream: false } }),
      token: 'private-fixture-token', bearer: 'private-fixture-bearer' };
  }

  async read() {
    this.readEntered.resolve();
    const index = this.reads++;
    if (this.onRead) return this.onRead();
    if (index < this.chunks.length) return { done: false, value: this.chunks[index] };
    if (this.onEof) this.onEof();
    return { done: true };
  }

  reader() {
    return {
      read: () => this.read(),
      cancel: async () => { this.cancelled++; if (this.onCancel) await this.onCancel(); },
      releaseLock: () => { this.released++; if (this.onRelease) this.onRelease(); },
    };
  }

  run() {
    const owner = this;
    const operation = script.runInNewContext({
      AbortController, TextDecoder, location: this.page,
      localStorage: { getItem: name => { assert.equal(name, '__cuid'); return this.device; } },
      document: { get cookie() { return owner.cookie; } },
      performance: { now: () => this.time },
      setTimeout: (callback, milliseconds) => {
        assert.equal(milliseconds, this.input.timeout_ms);
        this.expire = callback;
        return 1;
      },
      clearTimeout: id => { assert.equal(id, 1); this.cleared++; this.expire = null; },
      fetch: async (url, options) => {
        this.calls.push({ url, options });
        this.signal = options.signal;
        if (this.onFetch) return this.onFetch(url, options);
        return { status: this.status,
          headers: { get: name => { assert.equal(name, 'content-type'); return this.contentType; } },
          body: this.noBody ? null : { getReader: () => this.reader() } };
      },
    }, { timeout: 1000 });
    return operation(this.input);
  }
}

test('one fetch preserves the envelope and public caller credential mode', { timeout: 1000 }, async () => {
  for (const path of ['/ai/v3/responses/', '/ai/v3/messages/']) {
    const value = new BrowserFixture();
    value.input.path = path;
    const result = await value.run();
    assert.equal(value.calls.length, 1);
    const { url, options } = value.calls[0];
    assert.equal(url, 'https://api.framer.com' + path);
    assert.equal(options.method, 'POST');
    assert.equal(options.credentials, 'same-origin');
    assert.equal(options.redirect, 'error');
    assert.equal(options.cache, 'no-store');
    assert.deepEqual(JSON.parse(options.body), {
      ...JSON.parse(value.input.body), castleToken: value.input.token });
    assert.equal(options.headers.authorization, 'Bearer ' + value.input.bearer);
    assert.equal(options.headers['x-requested-by'], undefined);
    assert.deepEqual(Object.keys(options.headers).sort(), ['authorization', 'content-type']);
    assert.equal(options.headers['content-type'], 'application/json');
    assert.equal(Object.keys(options.headers).some(name => name.toLowerCase() === 'cookie'), false);
    assert.equal(result.status, 200);
    assert.equal(result.body, '{"id":"fixture-result"}');
    assert.equal(result.complete, true);
    assert.equal(result.started, 1);
    assert.equal(result.finished, 1);
    assert.equal(result.settled, true);
    assert.equal(result.failure, null);
    assert.equal(value.cancelled, 1);
    assert.equal(value.released, 1);
    assert.equal(value.cleared, 1);
    assert.equal(value.signal.aborted, true);
    assert.equal(JSON.stringify(result).includes('private-fixture'), false);
  }
});

test('a complete 403 remains one response for Python policy handling', { timeout: 1000 }, async () => {
  const value = new BrowserFixture();
  const body = '{"error":"Forbidden"}';
  value.status = 403;
  value.chunks = [utf8.encode(body)];
  const result = await value.run();
  assert.equal(value.calls.length, 1);
  assert.equal(result.status, 403);
  assert.equal(result.body, body);
  assert.equal(result.complete, true);
  assert.equal(result.failure, null);
  assert.equal(result.settled, true);
  assert.equal(result.finished, 1);
});

test('a rejected fetch exposes no raw exception or fabricated status', { timeout: 1000 }, async () => {
  const value = new BrowserFixture();
  value.onFetch = () => { throw new Error('private-fixture-network-error'); };
  const result = await value.run();
  assert.equal(value.calls.length, 1);
  assert.equal(result.status, null);
  assert.equal(result.body, '');
  assert.equal(result.failure, 'transport');
  assert.equal(result.complete, false);
  assert.equal(result.finished, 1);
  assert.equal(JSON.stringify(result).includes('private-fixture'), false);
});

test('result stays pending until the reader cancellation promise joins', { timeout: 1000 }, async () => {
  const value = new BrowserFixture();
  const entered = deferred();
  const release = deferred();
  let completed = false;
  value.onCancel = () => { entered.resolve(); return release.promise; };
  const pending = value.run().then(result => { completed = true; return result; });
  try {
    await entered.promise;
    assert.equal(completed, false);
    assert.equal(value.released, 0);
  } finally {
    release.resolve();
  }
  const result = await pending;
  assert.equal(result.settled, true);
  assert.equal(result.finished, 1);
  assert.equal(value.released, 1);
});

test('cancel or lock release failure never reports confirmed settlement', { timeout: 1000 }, async () => {
  for (const field of ['onCancel', 'onRelease']) {
    const value = new BrowserFixture();
    value[field] = () => { throw new Error('private-fixture-cleanup-error'); };
    const result = await value.run();
    assert.equal(result.failure, 'unconfirmed');
    assert.equal(result.settled, false);
    assert.equal(result.finished, 0);
    assert.equal(JSON.stringify(result).includes('private-fixture'), false);
  }
});

test('timeout during fetch retains one attempt and hides raw errors', { timeout: 1000 }, async () => {
  const value = new BrowserFixture();
  value.onFetch = (_, options) => new Promise((resolve, reject) => {
    options.signal.addEventListener('abort', () => reject(new Error('private-fixture-fetch-error')));
  });
  const pending = value.run();
  value.expire();
  const result = await pending;
  assert.equal(result.failure, 'timeout');
  assert.equal(result.status, null);
  assert.equal(result.started, 1);
  assert.equal(result.finished, 1);
  assert.equal(result.settled, true);
  assert.equal(value.calls.length, 1);
  assert.equal(JSON.stringify(result).includes('private-fixture'), false);
});

test('timeout during body reading waits for reader cancellation', { timeout: 1000 }, async () => {
  const value = new BrowserFixture();
  value.onRead = () => new Promise((resolve, reject) => {
    value.signal.addEventListener('abort', () => reject(new Error('private-fixture-read-error')));
  });
  const pending = value.run();
  await value.readEntered.promise;
  value.expire();
  const result = await pending;
  assert.equal(result.failure, 'timeout');
  assert.equal(result.complete, false);
  assert.equal(result.settled, true);
  assert.equal(value.cancelled, 1);
  assert.equal(value.released, 1);
  assert.equal(JSON.stringify(result).includes('private-fixture'), false);
});

test('an incomplete UTF-8 sequence cannot claim complete response bytes', { timeout: 1000 }, async () => {
  const value = new BrowserFixture();
  value.chunks = [Uint8Array.of(0xc3)];
  const result = await value.run();
  assert.equal(result.status, 200);
  assert.equal(result.complete, false);
  assert.equal(result.body, '');
  assert.equal(result.failure, 'transport');
  assert.equal(result.settled, true);
  assert.equal(value.calls.length, 1);
});

test('missing response body is a bounded failure without retry', { timeout: 1000 }, async () => {
  const value = new BrowserFixture();
  value.noBody = true;
  const result = await value.run();
  assert.equal(result.status, 200);
  assert.equal(result.failure, 'body');
  assert.equal(result.complete, false);
  assert.equal(result.settled, true);
  assert.equal(result.finished, 1);
  assert.equal(value.calls.length, 1);
  assert.equal(value.cancelled, 0);
});

test('split UTF-8 frames preserve exact text and byte count', { timeout: 1000 }, async () => {
  const value = new BrowserFixture();
  const text = '{"text":"λ🙂"}';
  const bytes = utf8.encode(text);
  value.chunks = Array.from(bytes, byte => Uint8Array.of(byte));
  value.input.byte_limit = bytes.length;
  const result = await value.run();
  assert.equal(result.body, text);
  assert.equal(result.bytes, bytes.length);
  assert.equal(result.complete, true);
  assert.equal(result.failure, null);
});

test('a chunk beyond the byte bound is discarded and settled once', { timeout: 1000 }, async () => {
  const value = new BrowserFixture();
  value.input.byte_limit = 4;
  value.chunks = [utf8.encode('abcd'), utf8.encode('e')];
  const result = await value.run();
  assert.equal(result.bytes, 4);
  assert.equal(result.body, '');
  assert.equal(result.complete, false);
  assert.equal(result.failure, 'response_limit');
  assert.equal(result.settled, true);
  assert.equal(result.finished, 1);
  assert.equal(value.calls.length, 1);
  assert.equal(value.cancelled, 1);
  assert.equal(value.released, 1);
});

test('two matching visible device cookies permit exactly one fetch', { timeout: 1000 }, async () => {
  const value = new BrowserFixture();
  value.cookie += '; __cuid=fixture-device';
  const result = await value.run();
  assert.equal(value.calls.length, 1);
  assert.equal(result.failure, null);
  assert.equal(result.status, 200);
  assert.equal(result.complete, true);
  assert.equal(result.started, 1);
  assert.equal(result.finished, 1);
  assert.equal(result.settled, true);
  assert.equal(value.cancelled, 1);
  assert.equal(value.released, 1);
});

test('a conflicting second visible device cookie refuses before fetch', { timeout: 1000 }, async () => {
  const value = new BrowserFixture();
  value.cookie += '; __cuid=conflicting-device';
  const result = await value.run();
  assert.equal(result.failure, 'context_changed');
  assert.equal(result.started, 0);
  assert.equal(result.finished, 0);
  assert.equal(result.settled, true);
  assert.equal(value.calls.length, 0);
  assert.equal(value.cleared, 1);
});

test('absent or empty visible device cookies refuse before fetch', { timeout: 1000 }, async () => {
  for (const cookie of ['', '__cuid=']) {
    const value = new BrowserFixture();
    value.cookie = cookie;
    const result = await value.run();
    assert.equal(result.failure, 'context_changed');
    assert.equal(result.started, 0);
    assert.equal(result.finished, 0);
    assert.equal(result.settled, true);
    assert.equal(value.calls.length, 0);
    assert.equal(value.cleared, 1);
  }
});

test('changed page or observed device prevents any fetch', { timeout: 1000 }, async () => {
  const changes = [value => { value.page.href += '?changed'; },
    value => { value.device = 'other'; }];
  for (const change of changes) {
    const value = new BrowserFixture();
    change(value);
    const result = await value.run();
    assert.equal(result.failure, 'context_changed');
    assert.equal(result.started, 0);
    assert.equal(result.finished, 0);
    assert.equal(result.settled, true);
    assert.equal(value.calls.length, 0);
    assert.equal(value.cleared, 1);
  }
});

test('device drift after EOF remains a failure', { timeout: 1000 }, async () => {
  const value = new BrowserFixture();
  value.onEof = () => { value.device = 'changed-after-response'; };
  const result = await value.run();
  assert.equal(result.complete, true);
  assert.equal(result.failure, 'context_changed');
  assert.equal(result.finished, 1);
  assert.equal(value.calls.length, 1);
});
