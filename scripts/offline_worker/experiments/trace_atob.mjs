import vm from 'node:vm';
import * as realm from './realm.mjs';
import * as pins from '../pins.mjs';
import * as col from '../collection.mjs';
import fs from 'node:fs';

const { context } = realm.buildRealm({ publicKey: pins.PUBLIC_KEY });
const mods = new Map();
for (const [role, [name]] of Object.entries(pins.MODULES)) {
  const bytes = fs.readFileSync(pins.ASSETS + '/' + name);
  const src = role === 'v3' ? col.collectionSource(bytes) : bytes.toString('utf8');
  mods.set(role, new vm.SourceTextModule(src, {
    context, identifier: pins.CDN + name,
    initializeImportMeta: meta => {
      if (role === 'v3') Object.assign(meta, { collect: v => v, sharedWorker: v => v, network: v => v });
    },
    importModuleDynamically: async s => mods.get(byRole(s))?.namespace,
  }));
}
function byRole(s) { for (const [r, [n]] of Object.entries(pins.MODULES)) if (s === pins.CDN + n) return r; throw new Error('imp ' + s); }
const wrapper = mods.get('wrapper');
await wrapper.link(async (spec, ref) => { const m = mods.get(byRole(spec)); if (m.status === 'unlinked') await m.link(arguments[0]); return m; });
try {
  await wrapper.evaluate({ timeout: 8000 });
  console.log('EVAL OK');
} catch (e) {
  console.log('FAILED message:', e.message);
  const st = (e.stack || '').split('\n');
  for (const line of st.slice(0, 5)) console.log('  ', line.trim());
}
