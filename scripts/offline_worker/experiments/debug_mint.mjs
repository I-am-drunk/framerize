// Capture the real error the Castle reporter hides during v3 mint.
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

// Neutralize the reporter so the raw error surfaces.
vm.runInContext(script, context, { filename: 'graph.bundle.js' });

// The v3 module registers a Sentry-style reporter. Find and disable it.
vm.runInContext(`
  if (globalThis.__castleDebug) globalThis.__castleDebug = null;
  // Patch console.error to capture instead of swallow
  const origError = console.error;
  globalThis.__capturedErrors = [];
  console.error = (...args) => {
    globalThis.__capturedErrors.push(args.map(a => {
      if (a instanceof Error) return a.message + '|' + (a.stack||'').split('\\n').slice(0,5).join('~');
      return String(a).slice(0,300);
    }).join(' '));
    origError.apply(console, args);
  };
`, context);

const wrapper = vm.runInContext('globalThis.__modules.wrapper', context);
const mint = wrapper.a;
console.log('minting...');
try {
  const token = await mint({ budget_ms: 8000 });
  console.log('TOKEN LENGTH:', token ? token.length : 'null');
  console.log('COLLECTED:', collected !== null, collected ? collected.length : 0);
} catch(e) {
  console.log('MINT THREW:', e.message);
  const errs = vm.runInContext('globalThis.__capturedErrors', context);
  if (errs) errs.forEach((e,i) => console.log('CAPTURED', i, e));
}
process.exit(0);
