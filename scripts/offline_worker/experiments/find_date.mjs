// Find what undefined object the SDK is accessing when it says 'Date'.
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

// Wrap window in a deep proxy that logs ALL property accesses
function deepProxy(obj, name, depth = 0) {
  if (depth > 3) return obj; // Prevent infinite recursion
  return new Proxy(obj, {
    get(target, prop) {
      const val = target[prop];
      if (val === undefined && typeof prop === 'string') {
        console.log('[undef]', name + '.' + prop);
      } else if (val && typeof val === 'object' && val !== null && !Array.isArray(val)) {
        return deepProxy(val, name + '.' + prop, depth + 1);
      }
      return val;
    },
  });
}

globals.window = deepProxy(globals.window, 'window');
globals.self = globals.window;
globals.globalThis = globals.window;

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
