// Capture every window-level property the pinned SDK touches during a mint.
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

// Proxy the realm's window/global before evaluation so every string
// property read and `in` check is recorded.
const touched = new Set();
const handler = {
  get(target, prop, receiver) {
    if (typeof prop === 'string') touched.add(prop);
    return Reflect.get(target, prop, receiver);
  },
};
const proxied = new Proxy(globals, handler);
const ctx = vm.createContext(proxied);

vm.runInContext(script, ctx, { filename: 'graph.bundle.js' });
const wrapper = vm.runInContext('globalThis.__modules.wrapper', ctx);
try { await wrapper.a({ budget_ms: 8000 }); } catch (e) { /* capture still valid */ }
const keys = [...touched].filter(k => !k.startsWith('has:')).sort();
console.log('window keys (' + keys.length + '): ' + keys.join(', '));
const hasKeys = [...touched].filter(k => k.startsWith('has:')).map(k => k.slice(4)).sort();
console.log('in-checks (' + hasKeys.length + '): ' + hasKeys.join(', '));
