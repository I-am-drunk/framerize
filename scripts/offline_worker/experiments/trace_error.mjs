// Capture the exact error with stack trace during createRequestToken.
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

// Call configure, then createRequestToken and capture the full error
const v3 = vm.runInContext('globalThis.__modules.v3', context);
const conf = v3.configure({pk: pins.PUBLIC_KEY, onError: (e) => {
  console.log('SDK ERROR:', e);
}});
console.log('configure returned');

const crt = conf.createRequestToken;
console.log('calling createRequestToken...');
try {
  const result = crt();
  console.log('createRequestToken returned:', typeof result);
  if (result && typeof result.then === 'function') {
    const token = await result;
    console.log('TOKEN:', token ? token.length : 'null');
  } else {
    console.log('result:', result);
  }
} catch(e) {
  console.log('CAUGHT:', e.message);
  console.log('STACK:', (e.stack || '').split('\n').slice(0, 10).join('\n'));
}
process.exit(0);
