// Mint in the no-JSDOM realm using classic-script graph evaluation. This
// avoids the Node vm ES-module realm record defect entirely.
import vm from 'node:vm';
import fs from 'node:fs';
import * as col from '../collection.mjs';
import * as pins from '../pins.mjs';
import * as realm from './realm.mjs';
import { buildGraphScript } from './graph_script.mjs';

const OUT = new URL('./script-mint-result.json', import.meta.url).pathname;
let collected = null;

async function main() {
  const globals = realm.buildRealmGlobals({ publicKey: pins.PUBLIC_KEY });
  const context = vm.createContext(globals);
  const script = buildGraphScript({
    assets: pins.ASSETS, modules: pins.MODULES, cdn: pins.CDN,
    collectionTransform: col.collectionSource,
  });
  fs.writeFileSync(new URL('./graph.bundle.js', import.meta.url), script);
  const result = { phase: 'evaluate', tokenLength: 0, collected: false, error: null };
  try {
    // Provide import.meta.collect to the v3 IIFE via a global hook the
    // collection transform can call. The transform rewrites to
    // import.meta.collect(Q); in script mode import.meta is unavailable, so
    // substitute a global collector before evaluation.
    const withCollect = script.replace(/import\.meta\.collect\(/g, 'globalThis.__collect(')
      .replace(/import\.meta\.sharedWorker\(/g, 'globalThis.__sharedWorker(')
      .replace(/import\.meta\.network\(/g, 'globalThis.__network(');
    globals.__collect = v => { collected = v; return v; };
    globals.__sharedWorker = v => v;
    globals.__network = v => v;
    try { vm.runInContext(withCollect, context, { filename: 'graph.bundle.js' }); }
    catch (e) { console.log('GRAPH EVAL THREW:', e.message.slice(0, 80)); throw e; }
    result.phase = 'mint';
    const wrapper = vm.runInContext('globalThis.__modules.wrapper', context);
    const mint = wrapper.a;
    const token = await Promise.race([
      mint({ budget_ms: 8000 }),
      new Promise((_, rej) => setTimeout(() => rej(new Error('mint-timeout')), 9000)),
    ]);
    result.tokenLength = token ? token.length : 0;
    result.collected = collected !== null;
    result.collectedLength = collected ? collected.length : 0;
    result.phase = 'done';
  } catch (e) {
    result.error = String(e && e.message || e);
    if (e && e.stack) result.stack = String(e.stack).split('\n').slice(0, 3);
  }
  fs.writeFileSync(OUT, JSON.stringify(result, null, 1));
  console.log('phase=' + result.phase, 'tokenLen=' + result.tokenLength, 'collected=' + result.collected, 'error=' + result.error);
}
main().catch(e => { console.error('FATAL', e && e.message); process.exit(1); });
