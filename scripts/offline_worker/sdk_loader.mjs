import fs from 'node:fs';
import vm from 'node:vm';
import { createHash } from 'node:crypto';
import { ASSETS, DOM, CDN, MODULES, SUPPORT, DOM_PINS, WRAPPER_MINT_EXPORT } from './pins.mjs';

export function verifyInputs() {
  const inputs = [...Object.values(MODULES).map(([name, , hash]) => [ASSETS + '/' + name, hash]),
    ...Object.values(SUPPORT), ...Object.entries(DOM_PINS).map(([name, hash]) => [DOM + '/' + name, hash])];
  const buffer = Buffer.allocUnsafe(64 * 1024);
  for (const [file, expected] of inputs) {
    const digest = createHash('sha256');
    const descriptor = fs.openSync(file, 'r');
    try {
      for (;;) {
        const count = fs.readSync(descriptor, buffer, 0, buffer.length, null);
        if (count === 0) break;
        digest.update(buffer.subarray(0, count));
      }
    } finally {
      fs.closeSync(descriptor);
    }
    if (digest.digest('hex') !== expected) throw new Error('input-identity');
  }
}

export class SdkModules {
  #modules = new Map(); #roles = new Map(); #evaluation;
  stats = { dynamic_imports: 0, v3_evaluated: false };
  constructor(context) {
    for (const [role, [name, , hash]] of Object.entries(MODULES)) {
      const bytes = fs.readFileSync(ASSETS + '/' + name);
      if (createHash('sha256').update(bytes).digest('hex') !== hash) throw new Error('input-identity');
      const module = new vm.SourceTextModule(bytes.toString('utf8'), {
        context, identifier: CDN + name,
        importModuleDynamically: (...args) => this.#dynamic(...args),
      });
      this.#modules.set(role, module); this.#roles.set(module, role);
    }
  }
  #attributes(attributes) {
    return attributes === undefined || (attributes !== null && typeof attributes === 'object'
      && Object.keys(attributes).length === 0);
  }
  #link(specifier, parent, extra) {
    const role = this.#roles.get(parent);
    if (!role || !this.#attributes(extra?.attributes) || !this.#attributes(extra?.assert))
      throw new Error('unexpected-import');
    const target = MODULES[role][1].find(name => specifier === CDN + MODULES[name][0]);
    if (!target) throw new Error('unexpected-import');
    return this.#modules.get(target);
  }
  async #dynamic(specifier, parent, attributes) {
    this.stats.dynamic_imports++;
    if (parent !== this.#modules.get('wrapper') || specifier !== CDN + MODULES.v3[0]
        || !this.#attributes(attributes)) throw new Error('unexpected-import');
    const module = this.#modules.get('v3');
    this.#evaluation ??= module.evaluate({ timeout: 5000 }).then(() => {
      this.stats.v3_evaluated = true;
    });
    await this.#evaluation; return module;
  }
  async prepare() {
    for (const [role, module] of this.#modules) {
      const expected = MODULES[role][1].map(name => CDN + MODULES[name][0]);
      if (module.dependencySpecifiers.length !== expected.length
          || !module.dependencySpecifiers.every((value, i) => value === expected[i]))
        throw new Error('dependency-set');
    }
    await this.#modules.get('wrapper').link((...args) => this.#link(...args));
    await this.#modules.get('v3').link((...args) => this.#link(...args));
  }
  async evaluate() {
    await this.#modules.get('wrapper').evaluate({ timeout: 5000 });
  }
  mint() {
    const wrapper = this.#modules.get('wrapper');
    if (wrapper.status !== 'evaluated' || typeof wrapper.namespace[WRAPPER_MINT_EXPORT] !== 'function')
      throw new Error('wrapper-unavailable');
    return wrapper.namespace[WRAPPER_MINT_EXPORT]();
  }
  loggerEntries() {
    const logger = this.#modules.get('logger');
    if (logger.status !== 'evaluated' || typeof logger.namespace.h !== 'function')
      throw new Error('logger-unavailable');
    return logger.namespace.h();
  }
}
