import vm from 'node:vm';
import * as realm from './realm.mjs';
import * as pins from '../pins.mjs';
import fs from 'node:fs';

const { context } = realm.buildRealm({ publicKey: pins.PUBLIC_KEY });
// Load only the wrapper module (which imports timeout, logger, bundler).
// This should trigger the same realm-record defect if it's a graph issue.
const mods = new Map();
for (const [role, [name]] of Object.entries(pins.MODULES)) {
  const bytes = fs.readFileSync(pins.ASSETS + '/' + name);
  mods.set(role, new vm.SourceTextModule(bytes.toString('utf8'), {
    context, identifier: pins.CDN + name,
    importModuleDynamically: async s => mods.get(byRole(s))?.namespace,
  }));
}
function byRole(s) { for (const [r, [n]] of Object.entries(pins.MODULES)) if (s === pins.CDN + n) return r; throw new Error('imp ' + s); }
for (const [, m] of mods) await m.link((...a) => linkOne(...a));
async function linkOne(spec, ref) { const r = byRole(spec); const m = mods.get(r); if (m.status === 'unlinked') await m.link(linkOne); return m; }
try {
  await mods.get('wrapper').evaluate({ timeout: 8000 });
  console.log('EVALUATED OK');
} catch (e) {
  console.log('EVAL FAILED:', e && e.message);
  console.log('stack head:', e && e.stack && e.stack.split('\n').slice(0,3).join(' | '));
}
