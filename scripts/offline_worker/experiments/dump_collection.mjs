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
const collector = col.createCollector();
globals.__collect = collector.collect;
globals.__sharedWorker = collector.sharedWorker;
globals.__network = collector.network;
let collected = null;
vm.runInContext(script, context, { filename: 'graph.bundle.js' });
const wrapper = vm.runInContext('globalThis.__modules.wrapper', context);
await wrapper.a({ budget_ms: 8000 }).catch(() => {});
// collected holds the JSON string returned by collector.collect via the wrapper
fs.writeFileSync('/tmp/realm-collection-string.txt', typeof collected === 'string' ? collected : JSON.stringify(collected));
fs.writeFileSync('/tmp/realm-collect-string.json', JSON.stringify({ typeof: typeof collected, sample: typeof collected === 'string' ? collected.slice(0,80) : null }));
console.log('slots=' + (collected ? collected.length : 0));
