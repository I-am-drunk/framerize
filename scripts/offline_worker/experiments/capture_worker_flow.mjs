// Capture what flows through the sharedWorker hook during a mint.
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
const swCalls = [];
globals.__collect = v => v;
globals.__sharedWorker = v => { swCalls.push(typeof v); return v; };
globals.__network = v => v;
vm.runInContext(script, context, { filename: 'graph.bundle.js' });
const wrapper = vm.runInContext('globalThis.__modules.wrapper', context);
try { await wrapper.a({ budget_ms: 8000 }); } catch (e) { /* capture still valid */ }
console.log('sharedWorker calls:', swCalls.length, JSON.stringify(swCalls.slice(0, 10)));
// Also: did the mint try to construct Worker/SharedWorker at all?
console.log('Worker in globals:', 'Worker' in globals, 'SharedWorker' in globals);
