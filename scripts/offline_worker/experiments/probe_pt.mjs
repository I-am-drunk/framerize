import vm from 'node:vm';
import * as realm from './realm.mjs';
import * as pins from '../pins.mjs';
import fs from 'node:fs';
const { context } = realm.buildRealm({ publicKey: pins.PUBLIC_KEY });
// Evaluate only the v3 module in isolation with a proxy to catch the read.
const bytes = fs.readFileSync(pins.ASSETS + '/castleSDKV3-LNGA36I3.mjs');
let collected;
const mod = new vm.SourceTextModule(bytes.toString('utf8'), {
  context, identifier: pins.CDN + 'castleSDKV3-LNGA36I3.mjs',
  initializeImportMeta: m => Object.assign(m, { collect: v => { collected = v; return v; }, sharedWorker: v => v, network: v => v }),
  importModuleDynamically: async s => { throw new Error('unexpected import ' + s); },
});
await mod.link(s => { throw new Error('link ' + s); });
try { await mod.evaluate({ timeout: 8000 }); console.log('v3 EVAL OK'); }
catch (e) { console.log('v3 FAILED:', e.message); }
