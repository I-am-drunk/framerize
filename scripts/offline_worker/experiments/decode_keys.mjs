// Decode the obfuscated document property names the v3 SDK uses.
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

// Instrument document to capture computed property names
const docKeys = new Set();
const origDoc = globals.document;
const docProxy = new Proxy(origDoc, {
  get(target, prop) {
    if (typeof prop === 'string') docKeys.add(prop);
    return target[prop];
  },
  has(target, prop) {
    if (typeof prop === 'string') docKeys.add(prop);
    return prop in target;
  },
});
globals.document = docProxy;
globals.window.document = docProxy;

// Run configure to trigger the SDK's environment detection
const v3 = vm.runInContext('globalThis.__modules.v3', context);
v3.configure({pk: pins.PUBLIC_KEY, onError: () => {}});

// Now dump all captured document property names
console.log('captured document keys:', Array.from(docKeys).sort().join(', '));
process.exit(0);

// Also capture during createRequestToken
const conf = v3.configure({pk: pins.PUBLIC_KEY, onError: () => {}});
const crt = conf.createRequestToken;
console.log('calling createRequestToken...');
try {
  const r = crt();
  if (r && typeof r.then === 'function') {
    await r;
  }
  console.log('createRequestToken succeeded');
} catch(e) {
  console.log('createRequestToken failed:', e.message.slice(0,100));
}
console.log('after createRequestToken, captured keys:', Array.from(docKeys).sort().join(', '));
process.exit(0);
