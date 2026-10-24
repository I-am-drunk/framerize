'use strict';
// node --experimental-vm-modules; exact resident JS, synthetic cached module.
// No public SDK, browser, fetch, token payload inspection, or network.
const assert = require('node:assert/strict');
const vm = require('node:vm');
const {fixture, args, mintSource, deferred} = require('./resident_crossover_js_tests.cjs');
const loggerUrl = 'https://app.framerstatic.com/chunk-VJ7UYMJI.mjs';
async function observed() {
  const f = fixture(), logger = {}, records = [];
  let next = 0, reads = 0, imports = 0;
  const add = (label, tags, owner = logger) => records.push({id: next++, logger: owner,
    parts: [label, 'synthetic-private-text', tags]});
  const module = new vm.SyntheticModule(['i', 'h'], function() {
    this.setExport('i', name => {assert.equal(name, 'castle'); return logger;});
    this.setExport('h', (...arguments_) => {
      assert.equal(arguments_.length, 0); reads++;
      if (f.brokenRead) throw Error('synthetic-private-read');
      return records;
    });
  }, {context: f.realm});
  await module.link(() => {throw Error('unexpected dependency');});
  await module.evaluate();
  f.mint = new vm.Script('(' + mintSource + ')', {importModuleDynamically: async url => {
    assert.equal(url, loggerUrl); imports++;
    if (f.beforeImport) await f.beforeImport();
    if (f.brokenImport) throw Error('synthetic-private-import');
    return module;
  }}).runInContext(f.realm);
  return Object.assign(f, {records, logger, add,
    counts: () => ({reads, imports})});
}

async function main() {
  const v3 = await observed();
  v3.add('Using Castle v2 token'); // Before this call, so excluded.
  v3.realm.afterMint = async () => {
    v3.add('Using Castle v3 token');
    v3.add('Using Castle v2 token', null, {}); // A different logger.
  };
  const result = await v3.mint(v3.pin, args());
  assert.equal(result.ok, true);
  assert.equal(result.sdk.complete, true);
  assert.equal(result.sdk.v3, 1);
  assert.equal(result.sdk.v2, 0);
  assert.equal(v3.realm.mintCount, 1);
  assert.deepEqual(v3.counts(), {reads: 2, imports: 1});
  assert.ok(!JSON.stringify(result.sdk).includes('private'));
  assert.ok(!JSON.stringify(result.sdk).includes(result.token));
  const v2 = await observed();
  v2.realm.afterMint = async () => {
    for (const castlePhase of ['module load', 'configuration', 'token generation', 'SDK'])
      for (const castleFailure of ['timeout', 'error'])
        v2.add('synthetic-private-error', {castleVersion: 'v3', castlePhase, castleFailure});
    v2.add('Using Castle v2 token');
  };
  const fallback = await v2.mint(v2.pin, args());
  assert.equal(fallback.sdk.v2, 1);
  assert.equal(fallback.sdk.v3, 0);
  for (const phase of ['module_load', 'configuration', 'token_generation', 'sdk'])
    for (const failure of ['timeout', 'error'])
      assert.equal(fallback.sdk[phase + '_' + failure], 1);
  assert.equal(v2.realm.mintCount, 1);
  const empty = await observed();
  const none = await empty.mint(empty.pin, args());
  assert.equal(none.sdk.complete, true);
  assert.equal(none.sdk.v3 + none.sdk.v2, 0);
  const lost = await observed();
  lost.add('before');
  lost.realm.afterMint = async () => {
    lost.records.length = 0;
    lost.add('Using Castle v3 token');
  };
  const partial = await lost.mint(lost.pin, args());
  assert.equal(partial.sdk.complete, false);
  assert.equal(partial.sdk.v3, 1);
  for (const failure of ['brokenRead', 'brokenImport']) {
    const f = await observed();
    f[failure] = true;
    assert.equal((await f.mint(f.pin, args())).sdk, null);
    assert.equal(f.realm.mintCount, 1);
  }
  const postRead = await observed();
  postRead.realm.afterMint = async () => {postRead.brokenRead = true;};
  assert.equal((await postRead.mint(postRead.pin, args())).sdk, null);
  assert.equal(postRead.realm.mintCount, 1);
  for (const id of [-1, 0.5, Number.MAX_SAFE_INTEGER + 1]) {
    const f = await observed();
    f.add('before'); f.records[0].id = id;
    assert.equal((await f.mint(f.pin, args())).sdk, null);
    assert.equal(f.realm.mintCount, 1);
  }
  const unknown = await observed();
  vm.runInContext(`__fzResidentWrapper = Object.freeze({...__fzResidentWrapper,
    url: 'https://app.framerstatic.com/chunk-UNKNOWN0.mjs'});`, unknown.realm);
  // An unsupported wrapper has no logger import or invented branch mapping.
  const unknownArgs = {...args(), wrapper_url: 'https://app.framerstatic.com/chunk-UNKNOWN0.mjs'};
  unknown.pin = {...unknown.pin, record: unknown.realm.__fzResidentWrapper};
  assert.equal((await unknown.mint(unknown.pin, unknownArgs)).sdk, null);
  assert.deepEqual(unknown.counts(), {reads: 0, imports: 0});
  assert.equal(unknown.realm.mintCount, 1);
  const moved = await observed();
  moved.beforeImport = async () => {moved.realm.document = {};};
  assert.equal((await moved.mint(moved.pin, args())).ok, false);
  assert.equal(moved.realm.mintCount, 0);
  const after = await observed();
  after.realm.afterMint = async () => {
    after.add('Using Castle v3 token'); after.realm.document = {};
  };
  const changed = await after.mint(after.pin, args());
  assert.equal(changed.ok, false);
  assert.equal(changed.sdk.v3, 1);
  const pending = await observed(), gate = deferred();
  pending.realm.afterMint = () => gate.promise;
  let finished = false;
  const work = pending.mint(pending.pin, args()).then(v => {finished = true; return v;});
  await new Promise(setImmediate);
  assert.equal(finished, false);
  assert.equal(pending.counts().reads, 1);
  pending.add('Using Castle v2 token'); gate.resolve();
  assert.equal((await work).sdk.v2, 1);
  process.stdout.write('resident SDK evidence exact-JS fixtures passed\n');
}
main().catch(() => {
  process.stderr.write('resident SDK evidence exact-JS fixtures failed\n');
  process.exitCode = 1;
});
