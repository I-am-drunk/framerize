// Mint a Castle token in the no-JSDOM realm. Uses the exact module flow
// proven to pass the vm realm record: build realm, create all modules,
// link the wrapper, evaluate the wrapper, dynamic-import resolves v3.
import vm from 'node:vm';
import fs from 'node:fs';
import * as col from '../collection.mjs';
import * as pins from '../pins.mjs';
import * as realm from './realm.mjs';

const OUT = new URL('./realm-mint-result.json', import.meta.url).pathname;
let collected = null;

async function main() {
  const globals = realm.buildRealmGlobals({ publicKey: pins.PUBLIC_KEY });
  const context = vm.createContext(globals);
  const mods = new Map();
  for (const [role, [name]] of Object.entries(pins.MODULES)) {
    const bytes = fs.readFileSync(pins.ASSETS + '/' + name);
    const src = role === 'v3' ? col.collectionSource(bytes) : bytes.toString('utf8');
    mods.set(role, new vm.SourceTextModule(src, {
      context, identifier: pins.CDN + name,
      initializeImportMeta: meta => {
        if (role === 'v3') Object.assign(meta, { collect: v => { collected = v; return v; }, sharedWorker: v => v, network: v => v });
      },
      importModuleDynamically: async s => {
        // Return the module's namespace. Node's dynamic-import machinery
        // links and evaluates it against the same context; manually doing
        // either trips the realm record defect.
        const mod = mods.get(byRole(s));
        return mod.namespace;
      },
    }));
  }
  function byRole(s) { for (const [r, [n]] of Object.entries(pins.MODULES)) if (s === pins.CDN + n) return r; throw new Error('imp ' + s); }

  const result = { phase: 'link', tokenLength: 0, collected: false, error: null };
  const wrapper = mods.get('wrapper');
  try {
    // Link the whole graph depth-first so every module (including v3 and its
    // bundler dependency) is linked before the wrapper's dynamic import fires.
    async function linkDeep(role) {
      const mod = mods.get(role);
      if (mod.status !== 'unlinked') return;
      await mod.link(s => mods.get(byRole(s)));
      const [_, deps] = pins.MODULES[role];
      for (const dep of deps) await linkDeep(dep);
    }
    await linkDeep('wrapper');
    // v3 is dynamically imported by the wrapper at mint time. It must be
    // linked AND evaluated before that dynamic import resolves, or Node
    // throws "Module status must not be unlinked or linking".
    await linkDeep('v3');
    await mods.get('v3').evaluate();
    result.phase = 'evaluate';
    await wrapper.evaluate();
    result.phase = 'mint';
    const mint = wrapper.namespace.a;
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
