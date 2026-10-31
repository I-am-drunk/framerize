import vm from 'node:vm';
import * as realm from './realm.mjs';
import * as pins from '../pins.mjs';
import fs from 'node:fs';
const { context, win } = realm.buildRealm({ publicKey: pins.PUBLIC_KEY });
// Wrap navigator.plugins to trace enumeration
const origPlugins = win.navigator.plugins;
win.navigator.plugins = new Proxy(origPlugins, {
  get(t, k) { console.log('[plugins.get]', String(k)); return t[k]; },
  has(t, k) { console.log('[plugins.has]', String(k)); return k in t; },
});
const mods = new Map();
for (const [role, [name]] of Object.entries(pins.MODULES)) {
  const bytes = fs.readFileSync(pins.ASSETS + '/' + name);
  mods.set(role, new vm.SourceTextModule(bytes.toString('utf8'), {
    context, identifier: pins.CDN + name,
    initializeImportMeta: m => { if (role === 'v3') Object.assign(m, { collect: v => v, sharedWorker: v => v, network: v => v }); },
    importModuleDynamically: async s => mods.get(byRole(s))?.namespace,
  }));
}
function byRole(s) { for (const [r, [n]] of Object.entries(pins.MODULES)) if (s === pins.CDN + n) return r; throw new Error('imp ' + s); }
async function linkDeep(role) { const m = mods.get(role); if (m.status !== 'unlinked') return; await m.link(s => mods.get(byRole(s))); for (const d of pins.MODULES[role][1]) await linkDeep(d); }
await linkDeep('wrapper');
try { await mods.get('wrapper').evaluate({ timeout: 8000 }); console.log('OK'); }
catch (e) { console.log('FAIL:', e.message); }
