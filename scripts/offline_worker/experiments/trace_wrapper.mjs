// Trace the wrapper's qX function to see where it hangs.
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

// Manually call the wrapper's qX function with tracing
const wrapper = vm.runInContext('globalThis.__modules.wrapper', context);

// First check if the wrapper's RX function returns the key
const key = vm.runInContext('window.castlePublishableAPIKey', context);
console.log('castlePublishableAPIKey:', key ? key.slice(0, 10) + '...' : 'undefined');

// Check if the v3 module was already configured
const v3 = vm.runInContext('globalThis.__modules.v3', context);
console.log('v3.configure type:', typeof v3.configure);

// Manually trigger the configuration
const conf = v3.configure({pk: key, onError: (e) => console.log('SDK ERROR:', e)});
console.log('configure returned, createRequestToken:', typeof conf.createRequestToken);

// Now try to call createRequestToken
const crt = conf.createRequestToken;
console.log('calling createRequestToken...');
try {
  const result = crt();
  console.log('createRequestToken returned:', typeof result);
  if (result && typeof result.then === 'function') {
    console.log('waiting for promise...');
    const token = await result;
    console.log('TOKEN:', token ? token.length : 'null');
  }
} catch(e) {
  console.log('CAUGHT:', e.message);
}
process.exit(0);
