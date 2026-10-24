'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const host = fs.readFileSync(path.join(__dirname, '../../scripts/camoufox_host.py'), 'utf8');
const url = 'data:text/javascript,' + encodeURIComponent('export async function a(){return "t".repeat(600)}\n//');
function source(name) {
  const match = host.match(new RegExp(name + ' = """([\\s\\S]*?)""" %'));
  assert.ok(match, 'host literal exists');
  return match[1].replaceAll('%s', url);
}
async function run(name, window) {
  return new Function('window', 'setTimeout', 'return ' + source(name))(
    window, callback => callback());
}
function checked(window) {
  const record = window.__fzResidentWrapper;
  assert.ok(Object.isFrozen(record));
  assert.equal(record.module, window.__fzCastleM);
  assert.equal(record.callable, window.__fzCastleM.a);
  assert.equal(record.entry, 'a');
  assert.equal(record.url, url);
  assert.equal(record.realm, window);
}
async function main() {
  const warm = {};
  assert.equal(await run('WARM_JS', warm), true);
  checked(warm);
  const mint = {};
  assert.equal((await run('MINT_JS', mint)).length, 600);
  checked(mint);
  const existing = {__fzCastleM: {a: async () => 't'.repeat(600)}};
  const old = existing.__fzCastleM;
  assert.equal(await run('WARM_JS', existing), true);
  assert.equal((await run('MINT_JS', existing)).length, 600);
  assert.equal(existing.__fzCastleM, old);
  assert.equal(existing.__fzResidentWrapper, undefined);
  let calls = 0;
  const fallback = {__fzCastleM: {a: async () => { calls++; return null; }}};
  assert.equal((await run('MINT_JS', fallback)).length, 600);
  assert.equal(calls, 3);
  const record = fallback.__fzResidentWrapper;
  assert.ok(Object.isFrozen(record));
  assert.equal(record.module, fallback.__fzCastleM);
  assert.equal(record.callable, fallback.__fzCastleM.a);
  assert.equal(record.realm, fallback);
  assert.ok(record.url.startsWith(url + '?cb='));
  assert.equal(record.entry, 'a');
  process.stdout.write('resident wrapper provenance cases passed\n');
}
main().catch(error => { console.error(error); process.exitCode = 1; });
