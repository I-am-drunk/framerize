import test from 'node:test';
import assert from 'node:assert/strict';
import { getEventListeners } from 'node:events';
import { inspect } from 'node:util';
import { createFramerBackend } from '../dist/framer-backend.js';
import { ProviderError } from '../dist/errors.js';
import { createServer } from '../dist/server.js';

const canary = 'synthetic-private-mint-diagnostic';
const codes = ['configuration', 'busy', 'closed', 'aborted', 'timeout', 'collection', 'encoder'];
const body = { model: 'fixture', messages: [{ role: 'user', content: 'hello' }], stream: true };

function request(signal = new AbortController().signal) {
  return { path: '/v1/chat/completions', signal, headers: {},
    body: (async function* () { yield Buffer.from(JSON.stringify(body)); })() };
}

function backendFor(context, mint, overrides = {}) {
  const backend = createFramerBackend({ projectId: 'fixture', origin: 'http://127.0.0.1:1',
    auth: { async accessToken() { return 'synthetic-access'; }, async close() {} },
    castle: { mint, async close() {} }, ...overrides });
  context.after(() => backend.close());
  return backend;
}

function assertPublicError(error) {
  assert.ok(error instanceof ProviderError);
  assert.equal(error.name, 'ProviderError');
  assert.equal(error.message, 'Framer provider unavailable');
  assert.equal(error.status, 503);
  assert.equal(error.code, 'credentials_unavailable');
  assert.equal(error.retryable, undefined);
}

async function rejected(context, reason, synchronous = false) {
  const controller = new AbortController();
  const backend = backendFor(context, () => {
    if (synchronous) throw reason;
    return Promise.reject(reason);
  });
  let failure;
  await assert.rejects(backend.forward(request(controller.signal)), error => {
    failure = error;
    return true;
  });
  assertPublicError(failure);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  assert.deepEqual(Object.keys(failure).sort(), ['code', 'name', 'status']);
  assert.equal(Object.hasOwn(failure, 'cause'), false);
  assert.doesNotMatch(inspect(failure, { showHidden: true }), new RegExp(canary));
  return failure;
}

function assertDiagnostic(error, code) {
  assert.deepEqual(error.diagnostic, { stage: 'mint', code });
  assert.equal(Object.isFrozen(error.diagnostic), true);
  assert.deepEqual(Object.getOwnPropertyDescriptor(error, 'diagnostic'), {
    value: error.diagnostic, enumerable: false, configurable: false, writable: false,
  });
  assert.deepEqual(Reflect.ownKeys(error).sort(), ['code', 'diagnostic', 'message', 'name', 'stack', 'status']);
  assert.doesNotMatch(JSON.stringify(error), /diagnostic|"stage"|"mint"/);
}

for (const code of codes) {
  test(`mint rejection retains only the allowlisted ${code} diagnostic`, { timeout: 2000 }, async context => {
    const reason = Object.assign(new Error(canary), { code, token: canary, path: canary });
    const error = await rejected(context, reason);
    assertDiagnostic(error, code);
    reason.code = 'changed';
    assert.equal(error.diagnostic.code, code);
    assert.throws(() => { error.diagnostic.code = 'changed'; }, TypeError);
    assert.throws(() => { error.diagnostic = {}; }, TypeError);
  });
}

test('synchronous mint rejection receives the same diagnostic', { timeout: 2000 }, async context => {
  assertDiagnostic(await rejected(context, { code: 'encoder' }, true), 'encoder');
});

const unknownReasons = [
  ['undefined', undefined], ['null', null], ['string', 'timeout'], ['number', 1], ['boolean', false],
  ['symbol', Symbol(canary)], ['array', []], ['function', () => {}],
  ['unrecognised code', { code: canary }], ['case mismatch', { code: 'TIMEOUT' }],
  ['collection deadline guess', { code: 'collection_timeout' }],
  ['whole-mint deadline guess', { code: 'mint_timeout' }],
  ['message and stack only', new Error('collection timeout encoder ' + canary)],
  ['inherited code', Object.create({ code: 'busy' })],
  ['boxed code', { code: new String('timeout') }],
];

for (const [label, reason] of unknownReasons) {
  test(`mint rejection uses unknown for ${label}`, { timeout: 2000 }, async context => {
    assertDiagnostic(await rejected(context, reason), 'unknown');
  });
}

test('hostile code accessors are not evaluated', { timeout: 2000 }, async context => {
  let accesses = 0;
  for (const throws of [false, true]) {
    const reason = Object.defineProperty({}, 'code', { get() {
      accesses += 1;
      if (throws) throw new Error(canary);
      return 'busy';
    } });
    const error = await rejected(context, reason);
    assert.equal(accesses, 0);
    assertDiagnostic(error, 'unknown');
  }
});

test('proxy and revoked proxy rejections remain unknown without traps', { timeout: 2000 }, async context => {
  let traps = 0;
  const reason = new Proxy({ code: 'busy' }, {
    get() { traps += 1; throw new Error(canary); },
    getOwnPropertyDescriptor() { traps += 1; throw new Error(canary); },
    getPrototypeOf() { traps += 1; throw new Error(canary); },
  });
  const error = await rejected(context, reason);
  assert.equal(traps, 0);
  assertDiagnostic(error, 'unknown');
  const revoked = Proxy.revocable({}, {});
  revoked.revoke();
  assertDiagnostic(await rejected(context, revoked.proxy), 'unknown');
});

test('mint diagnostics neither inspect other fields nor coerce codes', { timeout: 2000 }, async context => {
  let accesses = 0;
  const reason = Object.create(null);
  Object.defineProperty(reason, 'code', { value: 'timeout' });
  for (const key of ['message', 'stack', 'cause', 'token', 'path', 'then']) {
    Object.defineProperty(reason, key, { get() {
      accesses += 1;
      throw new Error(canary);
    } });
  }
  reason[Symbol(canary)] = canary;
  const error = await rejected(context, reason);
  assert.equal(accesses, 0);
  assertDiagnostic(error, 'timeout');
  const code = { [Symbol.toPrimitive]() { accesses += 1; throw new Error(canary); } };
  const unknown = await rejected(context, { code });
  assert.equal(accesses, 0);
  assertDiagnostic(unknown, 'unknown');
});

for (const action of ['abort', 'close', 'deadline']) {
  test(`mint diagnostics preserve ${action} precedence`, { timeout: 3000 }, async context => {
    const entered = Promise.withResolvers();
    const controller = new AbortController();
    const backend = backendFor(context, ({ signal }) => {
      entered.resolve();
      return new Promise((resolve, reject) => {
        signal.addEventListener('abort', () => reject({ code: 'encoder', message: canary }), { once: true });
      });
    }, { headersTimeoutMs: action === 'deadline' ? 150 : 2000 });
    const expected = action === 'abort' ? ['request_cancelled', 499]
      : action === 'close' ? ['provider_closed', 503] : ['headers_timeout', 504];
    const checked = assert.rejects(backend.forward(request(controller.signal)), error => {
      assert.ok(error instanceof ProviderError);
      assert.equal(error.code, expected[0]);
      assert.equal(error.status, expected[1]);
      assert.equal(error.retryable, undefined);
      assert.equal(Object.hasOwn(error, 'diagnostic'), false);
      assert.doesNotMatch(inspect(error, { showHidden: true }), new RegExp(canary));
      return true;
    });
    await entered.promise;
    if (action === 'abort') controller.abort();
    if (action === 'close') await backend.close();
    await checked;
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  });
}

test('credential rejections do not acquire a mint diagnostic', { timeout: 2000 }, async context => {
  let mints = 0;
  const backend = backendFor(context, () => { mints += 1; throw new Error(canary); }, {
    auth: { async accessToken() { throw { code: 'encoder', message: canary }; }, async close() {} },
  });
  await assert.rejects(backend.forward(request()), error => {
    assertPublicError(error);
    assert.equal(Object.hasOwn(error, 'diagnostic'), false);
    assert.doesNotMatch(inspect(error, { showHidden: true }), new RegExp(canary));
    return true;
  });
  assert.equal(mints, 0);
  assert.equal(Object.hasOwn(new ProviderError('ordinary failure'), 'diagnostic'), false);
});

test('mint diagnostics never change public errors or retry headers', { timeout: 5000 }, async context => {
  let reason;
  let failure;
  const backend = backendFor(context, async () => { throw reason; });
  const provider = { ...backend, async forward(input) {
    try { return await backend.forward(input); }
    catch (error) { failure = error; throw error; }
  } };
  const app = await createServer({ host: '127.0.0.1', port: 0, serverToken: 'synthetic-local' }, { provider });
  context.after(() => app.close());
  const address = await app.listen();
  const base = `http://127.0.0.1:${address.port}`;
  const cases = [
    ['/v1/chat/completions', body],
    ['/v1/chat/completions', { ...body, stream: false }],
    ['/openai/v1/responses', { model: 'fixture', input: 'hello', stream: false }],
    ['/anthropic/v1/messages', { ...body, max_tokens: 32, stream: false }],
  ];
  const failures = [];
  for (const code of ['busy', 'unknown']) {
    reason = { code: code === 'unknown' ? canary : code, message: canary, stack: canary };
    for (const [path, payload] of cases) {
      const response = await fetch(base + path, { method: 'POST',
        headers: { authorization: 'Bearer synthetic-local', 'content-type': 'application/json' },
        body: JSON.stringify(payload), signal: AbortSignal.timeout(2000) });
      const text = await response.text();
      assert.equal(response.status, 503);
      assert.equal(response.headers.get('x-should-retry'), null);
      assert.equal(response.headers.get('retry-after'), null);
      const error = { type: path === '/anthropic/v1/messages' ? 'api_error' : 'invalid_request_error',
        code: 'credentials_unavailable', message: 'Provider unavailable' };
      assert.deepEqual(JSON.parse(text), path === '/anthropic/v1/messages'
        ? { type: 'error', error } : { error });
      assert.doesNotMatch(text, /diagnostic|"stage"|synthetic-private/);
      assertPublicError(failure);
      failures.push({ failure, code });
    }
  }
  for (const { failure, code } of failures) assertDiagnostic(failure, code);
});
