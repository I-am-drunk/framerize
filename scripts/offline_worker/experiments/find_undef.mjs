// Find what undefined object the SDK is accessing.
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

// Wrap window in a Proxy to catch undefined property access chains
const winProxy = new Proxy(globals.window, {
  get(target, prop) {
    const val = target[prop];
    if (val === undefined && typeof prop === 'string' && prop !== 'selenium') {
      console.log('[undef]', prop);
    }
    return val;
  },
});
globals.window = winProxy;
globals.self = winProxy;
globals.globalThis = winProxy;

const v3 = vm.runInContext('globalThis.__modules.v3', context);
const conf = v3.configure({pk: pins.PUBLIC_KEY, onError: () => {}});
const crt = conf.createRequestToken;
console.log('calling createRequestToken...');
try {
  const result = crt();
  if (result && typeof result.then === 'function') {
    await result;
  }
} catch(e) {
  console.log('CAUGHT:', e.message);
}
process.exit(0);
