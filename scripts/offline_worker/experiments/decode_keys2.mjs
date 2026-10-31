// Capture ALL property accesses during createRequestToken, not just configure.
import vm from 'node:vm';
import * as col from '../collection.mjs';
import * as pins from '../pins.mjs';
import * as realm from './realm.mjs';
import { buildGraphScript } from './graph_script.mjs';

const globals = realm.buildRealmGlobals({ publicKey: pins.PUBLIC_KEY });
const context = vm.createContext(globals);
let script = buildGraphScript({
  assets: pins.ASSETS, modules: pins.MODULES, cdn: pins.CDN,
  collectionTransform: col.collectionSource,
});
script = script.replace(/import\.meta\.collect\(/g, 'globalThis.__collect(')
  .replace(/import\.meta\.sharedWorker\(/g, 'globalThis.__sharedWorker(')
  .replace(/import\.meta\.network\(/g, 'globalThis.__network(');
globals.__collect = v => v;
globals.__sharedWorker = v => v;
globals.__network = v => v;
vm.runInContext(script, context, { filename: 'graph.bundle.js' });

// Capture ALL string property accesses on document during the entire flow
const docKeys = new Set();
const origDoc = globals.document;
const handler = {
  get(target, prop, receiver) {
    if (typeof prop === 'string') docKeys.add(prop);
    return Reflect.get(target, prop, receiver);
  },
};
globals.document = new Proxy(origDoc, handler);
globals.window.document = globals.document;

// Also capture on window in case it goes through window.document
const origWin = globals.window;
const winHandler = {
  get(target, prop, receiver) {
    if (prop === 'document') return globals.document;
    return Reflect.get(target, prop, receiver);
  },
};
globals.window = new Proxy(origWin, winHandler);
globals.self = globals.window;
globals.globalThis = globals.window;

// Run the full wrapper mint
const wrapper = vm.runInContext('globalThis.__modules.wrapper', context);
const mint = wrapper.a;
console.log('minting...');
try {
  const token = await Promise.race([
    mint({ budget_ms: 5000 }),
    new Promise((_, rej) => setTimeout(() => rej(new Error('timeout')), 8000)),
  ]);
  console.log('TOKEN:', token ? token.length : 'null');
} catch(e) {
  console.log('ERROR:', e.message.slice(0,200));
}
console.log('all document keys:', Array.from(docKeys).sort().join(', '));
process.exit(0);
