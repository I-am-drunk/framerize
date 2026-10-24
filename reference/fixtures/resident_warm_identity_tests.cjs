'use strict';
// Exact embedded sources, synthetic logger and wrapper. No SDK or network.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const {fixture, args, deferred} = require('./resident_crossover_js_tests.cjs');
const source = fs.readFileSync(path.join(__dirname, '../../scripts/resident_authorization.py'), 'utf8');
const literal = name => source.match(new RegExp('^' + name + ' = r"""([\\s\\S]*?)"""', 'm'))[1];
const warm = literal('RESIDENT_WARM_IDENTITY_JS').replace('__RESIDENT_SDK_OBSERVER__', literal('RESIDENT_SDK_OBSERVER_JS'));
const device = '0123456789ab4def8123456789abcdef';
async function setup() {
  const f = fixture(), records = [], logger = {};
  let calls = 0, imports = 0;
  const module = new vm.SyntheticModule(['i', 'h'], function() {
    this.setExport('i', () => logger); this.setExport('h', () => records);
  }, {context: f.realm});
  await module.link(() => {throw Error('unexpected import');});
  await module.evaluate();
  f.importModule = async url => {
    assert.equal(url, 'https://app.framerstatic.com/chunk-VJ7UYMJI.mjs');
    imports++; if (f.importFailure) throw Error('synthetic'); return module;
  };
  f.warm = new vm.Script('(' + warm + ')', {importModuleDynamically: f.importModule}).runInContext(f.realm);
  f.identity = vm.runInContext('(' + literal('RESIDENT_SDK_IDENTITY_JS') + ')', f.realm);
  const capture = literal('RESIDENT_CAPTURE_JS').replace('__RESIDENT_PIN_CHECK__', literal('RESIDENT_PIN_JS'));
  f.capture = () => vm.runInContext('(' + capture + ')', f.realm)(args());
  f.call = async () => {
    calls++; if (f.callFailure) throw Error('synthetic');
    records.push({id: records.length, logger, parts: [f.label ?? 'Using Castle v2 token']});
    f.realm.localStorage.getItem = () => f.device ?? device;
    if (f.after) await f.after();
    return 't'.repeat(600);
  };
  f.run = () => f.warm(f.realm.__fzCastleM, args().wrapper_url, f.call);
  f.counts = () => ({calls, imports});
  return f;
}
async function main() {
  const f = await setup();
  await f.run();
  const record = f.realm.__fzResidentV2Device;
  assert.ok(Object.isFrozen(record));
  assert.equal(record.device, device);
  // The host's wire/storage device can differ from the cached SDK identity.
  f.realm.localStorage.getItem = () => args().device;
  f.pin = f.capture();
  assert.equal(f.identity(f.pin, args()), device);
  f.callFailure = true;
  await assert.rejects(f.run(), /synthetic/);
  assert.deepEqual(f.counts(), {calls: 2, imports: 1});
  assert.equal(f.realm.__fzResidentV2Device, record);
  for (const change of [
    'document = {}', 'localStorage = {getItem: () => "synthetic-device"}',
    '__fzCastleM = {...__fzCastleM}', '__fzCastleM.a = async () => "other"',
    '__fzResidentV2Device = Object.freeze({...__fzResidentV2Device})',
    'location.href += "?changed"',
  ]) {
    const altered = await setup(); await altered.run();
    altered.realm.localStorage.getItem = () => args().device;
    altered.pin = altered.capture();
    vm.runInContext(change, altered.realm);
    assert.equal(altered.identity(altered.pin, args()), null, change);
  }
  const missing = await setup();
  assert.equal(missing.identity(missing.pin, args()), null);
  missing.callFailure = true;
  await assert.rejects(missing.run(), /synthetic/);
  assert.deepEqual(missing.counts(), {calls: 1, imports: 1});
  assert.equal(missing.realm.__fzResidentV2Device, undefined);
  for (const setting of [
    {importFailure: true}, {label: 'Using Castle v3 token'},
    {device: '01234567-89ab-4def-8123-456789abcdef'},
  ]) {
    const rejected = Object.assign(await setup(), setting);
    assert.equal((await rejected.run()).length, 600);
    assert.equal(rejected.counts().calls, 1);
    assert.equal(rejected.realm.__fzResidentV2Device, undefined);
  }
  const pending = await setup(), gate = deferred();
  pending.after = () => gate.promise;
  const work = pending.run();
  await new Promise(setImmediate);
  assert.equal(pending.realm.__fzResidentV2Device, undefined);
  gate.resolve(); await work;
  assert.equal(pending.realm.__fzResidentV2Device.device, device);
  process.stdout.write('resident warm identity exact-JS fixtures passed\n');
}
module.exports = {setup};
if (require.main === module) {
  const deadline = setTimeout(() => {
    process.stderr.write('resident warm identity fixture did not complete\n');
    process.exitCode = 1;
  }, 5000);
  main().catch(error => {console.error(error); process.exitCode = 1;})
    .finally(() => clearTimeout(deadline));
}
