// Capture every property read on navigator, screen, and performance during a mint.
import vm from 'node:vm';
import * as col from '../collection.mjs';
import * as pins from '../pins.mjs';
import * as realm from './realm.mjs';
import { buildGraphScript } from './graph_script.mjs';

const globals = realm.buildRealmGlobals({ publicKey: pins.PUBLIC_KEY });
const touched = { navigator: new Set(), screen: new Set(), performance: new Set() };
for (const name of Object.keys(touched)) {
  globals[name] = new Proxy(globals[name], {
    get(target, prop, receiver) {
      if (typeof prop === 'string') touched[name].add(prop);
      return Reflect.get(target, prop, receiver);
    },
  });
}
globals.window.navigator = globals.navigator;
globals.window.screen = globals.screen;
globals.window.performance = globals.performance;

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
const wrapper = vm.runInContext('globalThis.__modules.wrapper', context);
try { await wrapper.a({ budget_ms: 8000 }); } catch (e) { /* capture still valid */ }
for (const [name, set] of Object.entries(touched))
  console.log(name + ' (' + set.size + '): ' + [...set].sort().join(', '));
