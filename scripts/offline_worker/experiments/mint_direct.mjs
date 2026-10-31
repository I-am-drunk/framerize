// Direct mint test without the script_mint.mjs wrapper.
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
const wrapper = vm.runInContext('globalThis.__modules.wrapper', context);
const mint = wrapper.a;
console.log('minting...');
try {
  const token = await Promise.race([
    mint({ budget_ms: 8000 }),
    new Promise((_, rej) => setTimeout(() => rej(new Error('timeout')), 12000)),
  ]);
  console.log('TOKEN LENGTH:', token ? token.length : 'null');
  console.log('COLLECTED:', collected !== null, collected ? collected.length : 0);
  if (collected) fs.writeFileSync('/tmp/realm-collection-string.txt', typeof collected === 'string' ? collected : JSON.stringify(collected));
} catch(e) {
  console.log('ERROR:', e.message.slice(0,200));
}
process.exit(0);
