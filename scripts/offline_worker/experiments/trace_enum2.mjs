import vm from 'node:vm';
import * as realm from './realm.mjs';
import * as pins from '../pins.mjs';
import * as col from '../collection.mjs';
import fs from 'node:fs';
const { context, win } = realm.buildRealm({ publicKey: pins.PUBLIC_KEY });
const mods = new Map();
for (const [role, [name]] of Object.entries(pins.MODULES)) {
  const bytes = fs.readFileSync(pins.ASSETS + '/' + name);
  const src = role === 'v3' ? col.collectionSource(bytes) : bytes.toString('utf8');
  mods.set(role, new vm.SourceTextModule(src, {
    context, identifier: pins.CDN + name,
    initializeImportMeta: m => { if (role === 'v3') Object.assign(m, { collect: v => v, sharedWorker: v => v, network: v => v }); },
    importModuleDynamically: async s => mods.get(byRole(s))?.namespace,
  }));
}
function byRole(s) { for (const [r, [n]] of Object.entries(pins.MODULES)) if (s === pins.CDN + n) return r; throw new Error('imp ' + s); }
async function linkDeep(role) { const m = mods.get(role); if (m.status !== 'unlinked') return; await m.link(s => mods.get(byRole(s))); for (const d of pins.MODULES[role][1]) await linkDeep(d); }
await linkDeep('wrapper');
try {
  await mods.get('wrapper').evaluate({ timeout: 8000 });
  const mint = mods.get('wrapper').namespace.a;
  const token = await mint({ budget_ms: 8000 });
  console.log('MINT OK len=', token ? token.length : 0);
} catch (e) { console.log('FAIL:', e.message); }
