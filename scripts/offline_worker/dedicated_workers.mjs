import { createPool } from './dedicated_pool.mjs';

export function installDedicatedWorkers(window, admittedHashes) {
  if (window.location.origin !== 'https://framer.com'
      || window.navigator.userAgent !== 'Framerize/0.1'
      || !Array.isArray(admittedHashes) || admittedHashes.length < 1
      || admittedHashes.length > 2 || admittedHashes.some(h => !/^[a-f0-9]{64}$/.test(h)))
    throw new Error('worker scope');
  const pool = createPool(window, new Set(admittedHashes));
  const urls = new Map(), listeners = new WeakMap();
  const create = window.URL.createObjectURL, revoke = window.URL.revokeObjectURL;
  let closed = false;
  class DedicatedWorker extends window.EventTarget {
    #slot;
    constructor(url, options) {
      super();
      if (closed || typeof url !== 'string' || options !== undefined || !urls.has(url))
        throw new window.DOMException('Unsupported worker URL or options', 'SecurityError');
      listeners.set(this, {});
      this.#slot = pool.start(urls.get(url), this);
    }
    postMessage(value) { this.#slot.send(value); }
    terminate() { this.#slot.stop(); }
  }
  for (const type of ['message', 'error']) Object.defineProperty(DedicatedWorker.prototype, 'on'+type, {
    get() { return listeners.get(this)[type] ?? null; },
    set(value) {
      const handlers = listeners.get(this);
      if (handlers[type]) this.removeEventListener(type, handlers[type]);
      handlers[type] = typeof value === 'function' ? value : null;
      if (handlers[type]) this.addEventListener(type, handlers[type]);
    },
  });
  const createObjectURL = blob => {
    if (closed || urls.size >= 4) throw new Error('worker URL limit');
    const url = create.call(window.URL, blob);
    urls.set(url, blob); return url;
  };
  const revokeObjectURL = url => {
    urls.delete(url); revoke.call(window.URL, url);
  };
  window.URL.createObjectURL = createObjectURL;
  window.URL.revokeObjectURL = revokeObjectURL;
  Object.defineProperty(window, 'Worker', { value: DedicatedWorker, configurable: true });
  return {
    Worker: DedicatedWorker,
    stats: pool.stats,
    replyStats: pool.replyStats,
    close() {
      closed = true; urls.clear();
      if (window.URL.createObjectURL === createObjectURL) window.URL.createObjectURL = create;
      if (window.URL.revokeObjectURL === revokeObjectURL) window.URL.revokeObjectURL = revoke;
      return pool.close();
    },
  };
}
