'use strict';
// UNRUN. Executes the exact Python-embedded JS in a synthetic vm realm only.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const python = fs.readFileSync(path.join(__dirname, '../../scripts/resident_authorization.py'), 'utf8');
const match = python.match(/RESIDENT_MINT_JS = r"""([\s\S]*?)"""/);
assert.ok(match, 'exact JS source must exist');
const source = match[1];
const wrapper = 'https://app.framerstatic.com/chunk-5LKSDTQ5.mjs';
const args = () => ({wrapper_url: wrapper, wrapper_export: 'a',
  page_url: 'https://framer.com/login', device: 'synthetic-device'});

function fixture(change = '') {
  const realm = vm.createContext({});
  vm.runInContext(`
    window = globalThis;
    document = {};
    location = {href: 'https://framer.com/login'};
    localStorage = {getItem: () => 'synthetic-device'};
    calls = 0;
    minted = 'synthetic-private-token';
    after = () => {};
    __fzCastleM = {a: async function() {
      if (this !== __fzCastleM) throw new Error('wrong receiver');
      calls++;
      await after();
      return minted;
    }};
    __fzResidentWrapper = Object.freeze({module: __fzCastleM,
      callable: __fzCastleM.a, url: '${wrapper}', entry: 'a', realm: window});
  `, realm);
  vm.runInContext(change, realm);
  return {realm, mint: vm.runInContext('(' + source + ')', realm)};
}

async function rejected(change, calls, code, input = args()) {
  const f = fixture(change);
  const result = await f.mint(input);
  assert.equal(result.ok, false);
  assert.equal(result.code, code);
  assert.deepEqual(Object.keys(result).sort(), ['code', 'ok']);
  assert.equal(f.realm.calls, calls);
}

async function main() {
  const valid = fixture();
  const result = await valid.mint(args());
  assert.equal(result.ok, true);
  assert.equal(result.token, 'synthetic-private-token');
  assert.equal(valid.realm.calls, 1);
  const cacheChanges = [
    'delete __fzResidentWrapper',
    '__fzResidentWrapper = {...__fzResidentWrapper}',
    '__fzResidentWrapper = Object.freeze({...__fzResidentWrapper, realm: {}})',
    '__fzResidentWrapper = Object.freeze({...__fzResidentWrapper, module: {}})',
    '__fzResidentWrapper = Object.freeze({...__fzResidentWrapper, callable: () => "fake"})',
    '__fzResidentWrapper = Object.freeze({...__fzResidentWrapper, get callable() {calls++; return () => "fake"}})',
    '__fzResidentWrapper = Object.freeze({...__fzResidentWrapper, entry: "b"})',
    '__fzResidentWrapper = Object.freeze({...__fzResidentWrapper, url: "https://wrong.invalid/"})',
    '__fzCastleM = {a: () => "fake"}',
    '__fzCastleM.a = () => "fake"',
  ];
  for (const change of cacheChanges) await rejected(change, 0, 'resident_cache');
  await rejected('location.href += "?changed"', 0, 'resident_context');
  await rejected('localStorage.getItem = () => "different"', 0, 'resident_context');
  await rejected('after = () => {throw new Error("synthetic-private-message")}', 1, 'resident_call');
  for (const change of ['location.href += "?changed"', 'document = {}',
    'localStorage = {getItem: () => "synthetic-device"}',
    'localStorage.getItem = () => "different"']) {
    await rejected('after = () => {' + change + '}', 1, 'resident_context');
  }
  for (const change of cacheChanges) {
    await rejected('after = () => {' + change + '}', 1, 'resident_cache');
  }
  const fallback = fixture('__fzResidentWrapper = Object.freeze({...__fzResidentWrapper, '
    + 'url: __fzResidentWrapper.url + "?cb=123"})');
  assert.equal((await fallback.mint({...args(), wrapper_url: wrapper + '?cb=123'})).ok, true);
  assert.equal(fallback.realm.calls, 1);
  await rejected('__fzResidentWrapper = Object.freeze({...__fzResidentWrapper, '
    + 'url: __fzResidentWrapper.url + "?cb=123"})', 0, 'resident_cache');
  for (const expression of ['null', 'true', '""', '"é".repeat(8193)',
    '"\\ud800"', '"\\udfff"', '"bad\\nvalue"']) {
    await rejected('minted = ' + expression, 1, 'resident_token');
  }
  const boundary = fixture('minted = "é".repeat(8192)');
  assert.equal((await boundary.mint(args())).ok, true);
  assert.equal(boundary.realm.calls, 1);
  process.stdout.write('resident authorization exact-JS cases passed\n');
}

main().catch(() => {
  process.stderr.write('resident authorization exact-JS fixture failed\n');
  process.exitCode = 1;
});
