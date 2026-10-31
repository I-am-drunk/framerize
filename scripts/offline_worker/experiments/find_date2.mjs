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

// Intercept the error to see the full stack
const origError = console.error;
let capturedStack = '';
console.error = (...args) => {
  if (args[0] && args[0].stack) capturedStack = args[0].stack;
  origError.apply(console, args);
};

const v3 = vm.runInContext('globalThis.__modules.v3', context);
const conf = v3.configure({pk: pins.PUBLIC_KEY, onError: (e) => {
  console.log('SDK ERROR:', e.message);
  if (e.stack) console.log('SDK STACK:', e.stack.split('\n').slice(0, 5).join('\n'));
}});
const crt = conf.createRequestToken;
console.log('calling createRequestToken...');
try {
  const result = crt();
  if (result && typeof result.then === 'function') {
    await result;
  }
} catch(e) {
  console.log('CAUGHT:', e.message);
  console.log('STACK:', (e.stack || '').split('\n').slice(0, 10).join('\n'));
}
process.exit(0);
