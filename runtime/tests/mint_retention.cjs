'use strict';
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const {Script, createContext} = require('node:vm');
const source = readFileSync(0, 'utf8');

async function main() {
  let imports = 0, calls = 0, fail = true;
  const failure = new Error('fixture SDK failure');
  const wrapper = {a: async () => {
    calls++;
    if (fail) throw failure;
    return 'fixture-token'.repeat(50);
  }};
  const context = createContext({setTimeout: callback => { callback(); return 1; }});
  context.window = {__fzCastleM: wrapper};
  const script = new Script(source, {importModuleDynamically: async () => {
    imports++;
    throw new Error('wrapper should stay resident');
  }});
  for (let i = 0; i < 100; i++) {
    await assert.rejects(script.runInContext(context), error => error === failure);
    assert.equal(context.window.__fzCastleM, wrapper);
  }
  assert.equal(imports, 0);
  assert.equal(calls, 300);
  fail = false;
  assert.equal(await script.runInContext(context), 'fixture-token'.repeat(50));
  assert.equal(calls, 301);
}
main().catch(error => { console.error(error); process.exitCode = 1; });
