// Intercept all document property accesses to find the missing method.
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

// Wrap document in a Proxy to log all property accesses
const origDoc = globals.document;
const docProxy = new Proxy(origDoc, {
  get(target, prop) {
    if (typeof prop === 'string' && !prop.startsWith('_')) {
      const val = target[prop];
      if (val === undefined) {
        console.log('[missing]', prop);
      } else if (typeof val === 'function') {
        console.log('[method]', prop);
      }
    }
    return target[prop];
  },
});
globals.document = docProxy;
globals.window.document = docProxy;

// Also wrap window to catch window.document accesses
const origWin = globals.window;
const winProxy = new Proxy(origWin, {
  get(target, prop) {
    if (prop === 'document') return docProxy;
    return target[prop];
  },
});
globals.window = winProxy;
globals.self = winProxy;
globals.globalThis = winProxy;

// Now run the wrapper mint
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
process.exit(0);
