// Classic-script evaluation of the pinned Castle graph for the no-JSDOM
// realm. Same pinned bytes and collection transform as sdk_loader, but the
// graph runs as one script so the SharedWorker collector captures the
// computed string (matching real Chrome), not the connection handshake.
import fs from 'node:fs';
import vm from 'node:vm';
import { createHash } from 'node:crypto';
import { collectionSource, createCollector } from './collection.mjs';
import { buildGraphScript } from './experiments/graph_script.mjs';
import { ASSETS, CDN, MODULES, WRAPPER_MINT_EXPORT } from './pins.mjs';

const ORDER = ['timeout', 'services', 'logger', 'bundler', 'v3', 'wrapper'];

export class ScriptModules {
  stats = { dynamic_imports: 1, v3_evaluated: true };
  #context; #collect = createCollector(); #script;
  constructor(context) {
    this.#context = context;
    for (const role of ORDER) {
      const [name, , hash] = MODULES[role];
      const bytes = fs.readFileSync(ASSETS + '/' + name);
      if (createHash('sha256').update(bytes).digest('hex') !== hash)
        throw new Error('input-identity');
    }
    this.#script = buildGraphScript({
      assets: ASSETS, modules: MODULES, cdn: CDN,
      collectionTransform: collectionSource,
    });
  }

  async prepare() {
    // Bind collector hooks directly onto the context global before the graph
    // runs, the same pattern the proven script_mint path uses.
    const bridge = this.#collect;
    const g = this.#context;
    g.__collect = v => bridge.collect(v);
    g.__sharedWorker = v => bridge.sharedWorker(v);
    g.__network = v => bridge.network(v);
  }
  async evaluate() {
    const withHooks = this.#script
      .replace(/import\.meta\.collect\(/g, 'globalThis.__collect(')
      .replace(/import\.meta\.sharedWorker\(/g, 'globalThis.__sharedWorker(')
      .replace(/import\.meta\.network\(/g, 'globalThis.__network(');
    vm.runInContext(withHooks, this.#context, { filename: 'graph.bundle.js', timeout: 5000 });
  }

  mint() {
    const wrapper = this.#context.__modules && this.#context.__modules.wrapper;
    if (!wrapper || typeof wrapper[WRAPPER_MINT_EXPORT] !== 'function')
      throw new Error('wrapper-unavailable');
    return wrapper[WRAPPER_MINT_EXPORT]();
  }
  loggerEntries() {
    const logger = this.#context.__modules && this.#context.__modules.logger;
    if (!logger || typeof logger.h !== 'function') throw new Error('logger-unavailable');
    return logger.h();
  }
}
