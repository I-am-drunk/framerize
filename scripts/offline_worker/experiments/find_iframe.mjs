// Track iframe creation and contentWindow access.
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

// Track iframe creation and contentWindow access
const origCreate = globals.document.createElement;
globals.document.createElement = function(tag) {
  const el = origCreate.call(this, tag);
  if (tag === 'iframe') {
    console.log('[iframe] created, contentWindow:', typeof el.contentWindow);
    // Wrap the element to track contentWindow access
    return new Proxy(el, {
      get(target, prop) {
        if (prop === 'contentWindow') {
          console.log('[iframe] contentWindow accessed:', typeof target[prop]);
        }
        return target[prop];
      },
    });
  }
  return el;
};

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
