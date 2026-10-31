// Trace where the wrapper mint stops inside the no-JSDOM realm.
import vm from 'node:vm';
import fs from 'node:fs';
import * as col from '../collection.mjs';
import * as pins from '../pins.mjs';
import * as realm from './realm.mjs';
import { buildGraphScript } from './graph_script.mjs';

let collected = null;
const globals = realm.buildRealmGlobals({ publicKey: pins.PUBLIC_KEY });
const context = vm.createContext(globals);
let script = buildGraphScript({
  assets: pins.ASSETS, modules: pins.MODULES, cdn: pins.CDN,
  collectionTransform: col.collectionSource,
});
script = script.replace(/import\.meta\.collect\(/g, 'globalThis.__collect(')
  .replace(/import\.meta\.sharedWorker\(/g, 'globalThis.__sharedWorker(')
  .replace(/import\.meta\.network\(/g, 'globalThis.__network(');
globals.__collect = v => { collected = v; return v; };
globals.__sharedWorker = v => v;
globals.__network = v => v;
vm.runInContext(script, context, { filename: 'graph.bundle.js' });

// Instrument the v3 module to trace calls
const v3 = vm.runInContext('globalThis.__modules.v3', context);
const origConf = v3.configure;
let confResult = null;
vm.runInContext(`
  globalThis.__modules.v3.configure = function(opts) {
    console.log('[trace] configure called with pk:', opts.pk ? opts.pk.slice(0,8) + '...' : 'none');
    globalThis.__confResult = (${origConf.toString()})(opts);
    console.log('[trace] configure returned, createRequestToken:', typeof globalThis.__confResult.createRequestToken);
    return globalThis.__confResult;
  };
`, context);

// Also trace createRequestToken
vm.runInContext(`
  if (globalThis.__confResult && globalThis.__confResult.createRequestToken) {
    const origCrt = globalThis.__confResult.createRequestToken;
    globalThis.__confResult.createRequestToken = function(...args) {
      console.log('[trace] createRequestToken called');
      const r = origCrt.apply(this, args);
      console.log('[trace] createRequestToken returned:', typeof r, r ? r.length : 'null');
      return r;
    };
  }
`, context);

// Run the wrapper mint with a timeout
const wrapper = vm.runInContext('globalThis.__modules.wrapper', context);
const mint = wrapper.a;
console.log('minting...');
try {
  const token = await Promise.race([
    mint({ budget_ms: 5000 }),
    new Promise((_, rej) => setTimeout(() => rej(new Error('timeout')), 8000)),
  ]);
  console.log('TOKEN LENGTH:', token ? token.length : 'null');
} catch(e) {
  console.log('MINT ERROR:', e.message.slice(0,100));
}
process.exit(0);
