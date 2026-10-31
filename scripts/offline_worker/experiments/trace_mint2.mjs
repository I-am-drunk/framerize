// Check if createRequestToken is async and what it returns.
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

// Call configure and inspect the result
const v3 = vm.runInContext('globalThis.__modules.v3', context);
const conf = v3.configure({pk: pins.PUBLIC_KEY, onError: (e) => console.log('SDK ERROR:', e)});
console.log('configure returned keys:', Object.keys(conf));

const crt = conf.createRequestToken;
console.log('createRequestToken type:', typeof crt);
console.log('createRequestToken constructor:', crt.constructor.name);
console.log('createRequestToken async:', crt.constructor.name === 'AsyncFunction');

// Try calling it
try {
  const result = crt();
  console.log('createRequestToken returned:', typeof result);
  if (result && typeof result.then === 'function') {
    console.log('result is a promise');
    result.then(t => console.log('promise resolved:', typeof t, t ? t.length : 'null'))
          .catch(e => console.log('promise rejected:', e.message));
  } else {
    console.log('result value:', result ? String(result).slice(0,50) : 'null');
  }
} catch(e) {
  console.log('createRequestToken threw:', e.message.slice(0,100));
}
