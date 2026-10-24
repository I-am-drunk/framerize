'use strict';
// UNRUN. Exact embedded JavaScript in a synthetic realm, never a browser.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const python = fs.readFileSync(path.join(__dirname, '../../scripts/resident_authorization.py'), 'utf8');
function literal(name) {
  const match = python.match(new RegExp('^' + name + ' = r"""([\\s\\S]*?)"""', 'm'));
  assert.ok(match, name);
  return match[1];
}
const pinSource = literal('RESIDENT_PIN_JS');
const captureSource = literal('RESIDENT_CAPTURE_JS').replace('__RESIDENT_PIN_CHECK__', pinSource);
const originalMint = literal('RESIDENT_MINT_JS');
const mintSource = literal('RESIDENT_PINNED_MINT_JS').replace('__RESIDENT_MINT__', originalMint)
  .replace('__RESIDENT_SDK_OBSERVER__', literal('RESIDENT_SDK_OBSERVER_JS'));
const fetchSource = literal('RESIDENT_FETCH_JS');
const wrapper = 'https://app.framerstatic.com/chunk-5LKSDTQ5.mjs';
const args = () => ({wrapper_url: wrapper, wrapper_export: 'a',
  page_url: 'https://framer.com/login', device: 'synthetic-device'});
const success = JSON.stringify({id: 'fixture', status: 'completed',
  output: [{type: 'message', content: [{type: 'output_text', text: 'fixture'}]}]});
function deferred() {
  let resolve;
  const promise = new Promise(done => {resolve = done;});
  return {promise, resolve};
}
function fixture(change = '') {
  const realm = vm.createContext({TextEncoder, TextDecoder, AbortController,
    setTimeout, clearTimeout, performance, Buffer, responseText: success});
  vm.runInContext(`
    window = globalThis; document = {};
    location = {href: 'https://framer.com/login'};
    localStorage = {getItem: () => 'synthetic-device'};
    calls = []; mintCount = 0; cancelCount = 0; released = false;
    afterMint = async () => {}; afterFetch = async () => {};
    beforeRead = async () => {}; cancelBody = async () => {};
    __fzCastleM = {a: async function() {
      if (this !== __fzCastleM) throw new Error('wrong receiver');
      mintCount++; await afterMint(); return 'synthetic-token-' + mintCount;
    }};
    __fzResidentWrapper = Object.freeze({module: __fzCastleM,
      callable: __fzCastleM.a, url: '${wrapper}', entry: 'a', realm: window});
    fetch = async (url, options) => {
      calls.push({url, options}); await afterFetch(); let reads = 0;
      return {status: 200, headers: {get: () => 'application/json'}, body: {
        getReader: () => ({read: async () => {
          await beforeRead();
          return reads++ ? {done: true} : {done: false,
            value: new TextEncoder().encode(responseText)};
        }, cancel: async () => {cancelCount++; await cancelBody();},
        releaseLock: () => {released = true;}})
      }};
    };
  `, realm);
  vm.runInContext(change, realm);
  const capture = vm.runInContext('(' + captureSource + ')', realm);
  return {realm, pin: capture(args()),
    check: vm.runInContext('(' + pinSource + ')', realm),
    mint: vm.runInContext('(' + mintSource + ')', realm),
    fetch: vm.runInContext('(' + fetchSource + ')', realm)};
}
function input(token = 'synthetic-token-1') {
  return {...args(), path: '/ai/v3/openai/responses/', bearer: 'synthetic-bearer',
    body: JSON.stringify({projectId: 'fixture', request: {stream: false}}),
    token, timeout_ms: 1000};
}
async function main() {
  const f = fixture();
  assert.equal(f.check(f.pin, args()), true);
  assert.equal(f.realm.mintCount, 0);
  assert.equal(f.realm.calls.length, 0);
  const first = await f.mint(f.pin, args());
  const baseline = await f.fetch(f.pin, input(first.token));
  assert.equal(baseline.complete, true);
  assert.equal(baseline.settled, true);
  assert.equal(baseline.failure, null);
  const second = await f.mint(f.pin, args());
  assert.notEqual(first.token, second.token);
  assert.equal(f.realm.mintCount, 2);
  assert.equal(f.realm.calls.length, 1);
  const {url, options} = f.realm.calls[0];
  assert.equal(url, 'https://api.framer.com/ai/v3/openai/responses/');
  assert.equal(options.method, 'POST');
  assert.equal(options.credentials, 'include');
  assert.equal(options.redirect, 'error');
  assert.equal(options.headers['Content-Type'], 'application/json; charset=utf-8');
  assert.equal(options.headers['X-Requested-By'], 'Framer');
  assert.equal(options.headers.Authorization, 'Bearer synthetic-bearer');
  assert.equal(JSON.parse(options.body).castleToken, first.token);
  assert.equal(f.realm.cancelCount, 1);
  assert.equal(f.realm.released, true);
  for (const change of [
    'location.href += "?drift"',
    'localStorage.getItem = () => "different-device"',
    'document = {}',
    'localStorage = {getItem: () => "synthetic-device"}',
    '__fzResidentWrapper = Object.freeze({...__fzResidentWrapper})',
    '__fzCastleM = {a: __fzCastleM.a}',
    '__fzCastleM.a = async () => "fake"',
  ]) {
    const altered = fixture();
    vm.runInContext(change, altered.realm);
    assert.equal(altered.check(altered.pin, args()), false);
    assert.equal((await altered.mint(altered.pin, args())).ok, false);
    assert.equal(altered.realm.mintCount, 0);
    const result = await altered.fetch(altered.pin, input());
    assert.equal(result.started, 0);
    assert.equal(result.failure, 'context_changed');
  }
  const malformed = fixture('delete __fzResidentWrapper');
  assert.equal(malformed.check(malformed.pin, args()), false);
  assert.equal((await malformed.mint(malformed.pin, args())).ok, false);
  for (const change of ['document = {}', 'location.href += "?drift"',
    '__fzResidentWrapper = Object.freeze({...__fzResidentWrapper})']) {
    const altered = fixture('afterMint = async () => {' + change + '}');
    assert.equal((await altered.mint(altered.pin, args())).ok, false);
    assert.equal(altered.realm.mintCount, 1);
  }
  const noAccessor = fixture('__fzResidentWrapper = Object.freeze({' +
    'get callable() {mintCount++; throw new Error("getter");}})');
  assert.equal(noAccessor.check(noAccessor.pin, args()), false);
  assert.equal(noAccessor.realm.mintCount, 0);
  const failed = fixture('afterFetch = async () => {throw new Error("private")};');
  const failure = await failed.fetch(failed.pin, input());
  assert.equal(failure.failure, 'transport');
  assert.equal(failure.complete, false);
  assert.equal(failure.settled, true);
  assert.equal(failed.realm.calls.length, 1);
  assert.equal(failed.realm.mintCount, 0);
  const oversized = fixture('responseText = "x".repeat(65537)');
  const capped = await oversized.fetch(oversized.pin, input());
  assert.equal(capped.failure, 'response_limit');
  assert.equal(capped.body, '');
  assert.equal(capped.complete, false);
  assert.equal(capped.settled, true);
  assert.equal(oversized.realm.cancelCount, 1);
  const boundary = fixture('responseText = "x".repeat(65536)');
  const full = await boundary.fetch(boundary.pin, input());
  assert.equal(full.bytes, 65536);
  assert.equal(full.complete, true);
  const uncertain = fixture('cancelBody = async () => {throw new Error("private")};');
  const unknown = await uncertain.fetch(uncertain.pin, input());
  assert.equal(unknown.failure, 'unconfirmed');
  assert.equal(unknown.settled, false);
  assert.equal(unknown.finished, 0);
  const drift = fixture('cancelBody = async () => {location.href += "?drift"};');
  const changed = await drift.fetch(drift.pin, input());
  assert.equal(changed.failure, 'context_changed');
  assert.equal(changed.settled, true);
  for (const step of ['beforeRead', 'cancelBody']) {
    const held = fixture();
    const gate = deferred();
    held.realm[step] = () => gate.promise;
    let finished = false;
    const work = held.fetch(held.pin, input()).then(result => {finished = true; return result;});
    await new Promise(setImmediate);
    assert.equal(finished, false);
    assert.equal(held.realm.calls.length, 1);
    gate.resolve();
    assert.equal((await work).settled, true);
    assert.equal(held.realm.released, true);
  }
  const timeout = fixture(`fetch = async (url, options) => {
    calls.push({url, options});
    return await new Promise((resolve, reject) => {
      options.signal.addEventListener('abort', () => reject(new Error('aborted')));
    });
  }`);
  const expired = await timeout.fetch(timeout.pin, {...input(), timeout_ms: 5});
  assert.equal(expired.failure, 'timeout');
  assert.equal(expired.settled, true);
  assert.equal(timeout.realm.calls.length, 1);
  const injected = fixture();
  const invalid = await injected.fetch(injected.pin, {...input(),
    body: JSON.stringify({castleToken: 'injected'})});
  assert.equal(invalid.started, 0);
  assert.equal(injected.realm.calls.length, 0);
  process.stdout.write('resident crossover exact-JS fixtures passed\n');
}
module.exports = {fixture, args, mintSource, deferred};
if (require.main === module) main().catch(() => {
  process.stderr.write('resident crossover exact-JS fixtures failed\n');
  process.exitCode = 1;
});
