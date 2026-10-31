// Run the v3 configure step explicitly before the wrapper mint.
import vm from 'node:vm';
import fs from 'node:fs';
import * as col from '../collection.mjs';
import * as pins from '../pins.mjs';
import * as realm from './realm.mjs';
import { buildGraphScript } from './graph_script.mjs';

let collected = null;
const globals = realm.buildRealmGlobals({ publicKey: pins.PUBLIC_KEY });
const context = vm.createContext(globals);
let script = buildGraphScript({
  assets: pins.ASSETS, modules: pins.MODULES, cdn: pins.CDN,
  collectionTransform: col.collectionSource,
});
script = script.replace(/import\.meta\.collect\(/g, 'globalThis.__collect(')
  .replace(/import\.meta\.sharedWorker\(/g, 'globalThis.__sharedWorker(')
  .replace(/import\.meta\.network\(/g, 'globalThis.__network(');
globals.__collect = v => { collected = v; return v; };
globals.__sharedWorker = v => v;
globals.__network = v => v;
vm.runInContext(script, context, { filename: 'graph.bundle.js' });

// v3 exposes { configure }. Call it first, then wrapper.a.
const v3 = vm.runInContext('globalThis.__modules.v3', context);
console.log('v3.configure type:', typeof v3.configure);

// The wrapper calls v3.configure internally. Check if it awaits it.
// Look at the wrapper source to see the call pattern.
const wrapperSrc = fs.readFileSync(pins.ASSETS + '/' + pins.MODULES.wrapper[0], 'utf8');
const confCall = wrapperSrc.match(/configure\s*\(/g);
console.log('configure calls in wrapper:', confCall ? confCall.length : 0);

// Check what the wrapper's mint function actually does by reading the tail
const tail = wrapperSrc.slice(-800);
console.log('wrapper tail:', tail.slice(0, 400));
