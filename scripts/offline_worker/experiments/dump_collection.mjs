// Dump the realm's 765-slot collection array to JSON for offline diffing.
import vm from 'node:vm';
import * as col from '../collection.mjs';
import * as pins from '../pins.mjs';
import * as realm from './realm.mjs';
import { buildGraphScript } from './graph_script.mjs';
import fs from 'node:fs';

const graphScript = buildGraphScript({
  assets: pins.ASSETS, modules: pins.MODULES, cdn: pins.CDN,
  collectionTransform: col.collectionSource,
});
const globals = realm.buildRealmGlobals({ publicKey: pins.PUBLIC_KEY });
const context = vm.createContext(globals);
const script = graphScript
  .replace(/import\.meta\.collect\(/g, 'globalThis.__collect(')
  .replace(/import\.meta\.sharedWorker\(/g, 'globalThis.__sharedWorker(')
  .replace(/import\.meta\.network\(/g, 'globalThis.__network(');
let collected = null;
globals.__collect = v => { collected = v; return v; };
globals.__sharedWorker = v => v;
globals.__network = v => v;
vm.runInContext(script, context, { filename: 'graph.bundle.js' });
const wrapper = vm.runInContext('globalThis.__modules.wrapper', context);
await wrapper.a({ budget_ms: 8000 }).catch(() => {});
fs.writeFileSync('/tmp/realm-collection.json', JSON.stringify(collected));
console.log('slots=' + (collected ? collected.length : 0));
