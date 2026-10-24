'use strict';
// stdin contains real observe_warm_identity(WARM_JS/MINT_JS) from Python.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const {setup} = require('./resident_warm_identity_tests.cjs');
const scripts = JSON.parse(fs.readFileSync(0, 'utf8'));
async function main() {
  for (const name of ['WARM_JS', 'MINT_JS']) {
    const f = await setup();
    f.realm.__fzCastleM.a = async function() {
      assert.equal(this, f.realm.__fzCastleM);
      return f.call();
    };
    vm.runInContext('__fzResidentWrapper = Object.freeze({...__fzResidentWrapper, callable: __fzCastleM.a})', f.realm);
    const run = () => new vm.Script(scripts[name], {
      importModuleDynamically: f.importModule,
    }).runInContext(f.realm);
    const value = await run();
    assert.equal(name === 'WARM_JS' ? value : value.length === 600, true);
    assert.equal(f.counts().calls, 1);
    assert.equal(f.realm.__fzResidentV2Device.device, '0123456789ab4def8123456789abcdef');
    await run();
    assert.deepEqual(f.counts(), {calls: 2, imports: 1});
  }
  process.stdout.write('resident host warm/mint decoration passed\n');
}
const deadline = setTimeout(() => {
  process.stderr.write('resident host warm/mint fixture did not complete\n');
  process.exitCode = 1;
}, 5000);
main().catch(error => {console.error(error); process.exitCode = 1;})
  .finally(() => clearTimeout(deadline));
