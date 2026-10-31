// Use the SDK's own error handler to capture the real error.
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

// The SDK has an onError callback. Use it to capture the real error.
let sdkError = null;
const v3 = vm.runInContext('globalThis.__modules.v3', context);
const conf = v3.configure({
  pk: pins.PUBLIC_KEY,
  onError: (e) => {
    sdkError = e;
    console.log('SDK ERROR:', e.message || e);
    if (e.cause) console.log('SDK CAUSE:', e.cause.message || e.cause);
    if (e.stack) console.log('SDK STACK:', e.stack.split('\n').slice(0, 5).join('\n'));
  }
});
console.log('configure returned');

// Now call createRequestToken
const crt = conf.createRequestToken;
console.log('calling createRequestToken...');
try {
  const result = crt();
  console.log('createRequestToken returned:', typeof result);
  if (result && typeof result.then === 'function') {
    const token = await result;
    console.log('TOKEN:', token ? token.length : 'null');
  }
} catch(e) {
  console.log('CAUGHT:', e.message);
  if (sdkError) console.log('SDK error was:', sdkError.message || sdkError);
}
process.exit(0);
