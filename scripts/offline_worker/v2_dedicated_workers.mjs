import { Blob, Buffer } from 'node:buffer';
import { createHash } from 'node:crypto';
import { URL } from 'node:url';
import { Worker as Thread } from 'node:worker_threads';

const createURL = URL.createObjectURL.bind(URL);
const revokeURL = URL.revokeObjectURL.bind(URL);
const blobSize = Object.getOwnPropertyDescriptor(Blob.prototype, 'size').get;
const blobBytes = Blob.prototype.arrayBuffer;
const installed = new WeakSet();
const digest = bytes => createHash('sha256').update(bytes).digest('hex');

// Dormant until explicitly installed by the owner of the V2 window.
export function installV2DedicatedWorkers(window, admittedScriptSha256) {
  if (window?.location?.origin !== 'https://framer.com' || installed.has(window)
      || typeof admittedScriptSha256 !== 'string'
      || !/^[a-f0-9]{64}$/.test(admittedScriptSha256)) throw new Error('worker scope');
  const urls = new Map(), slots = new Set(), listeners = new WeakMap();
  const counts = { started: 0, joined: 0, errors: 0, messages: 0 };
  let closed = false, closing;
  function fail(slot) {
    if (slot.failed || slot.stopped || closed) return;
    slot.failed = true; counts.errors++;
    slot.target.dispatchEvent(new window.ErrorEvent('error', { message: 'worker failed' }));
  }
  function start(blob, target) {
    if (slots.size) throw new window.DOMException('Worker limit', 'QuotaExceededError');
    const slot = { target, worker: null, stopped: false, failed: false, sent: false };
    slots.add(slot);
    slot.ready = (async () => {
      const bytes = Buffer.from(await blobBytes.call(blob));
      const source = bytes.toString('utf8');
      if (!bytes.length || bytes.length > 4096 || !Buffer.from(source).equals(bytes)
          || digest(bytes) !== admittedScriptSha256) throw new Error('worker source');
      if (slot.stopped || closed) { slots.delete(slot); return; }
      const worker = slot.worker = new Thread(new URL('./v2_dedicated_runtime.mjs', import.meta.url), {
        workerData: { source, sha256: admittedScriptSha256 }, execArgv: [],
        resourceLimits: { maxOldGenerationSizeMb: 16, maxYoungGenerationSizeMb: 4, stackSizeMb: 2 },
        stdout: true, stderr: true,
      });
      counts.started++;
      slot.joined = new Promise(resolve => worker.once('exit', () => {
        slot.exited = true; counts.joined++; slots.delete(slot);
        resolve(); if (!slot.stopped) fail(slot);
      }));
      worker.on('error', () => fail(slot));
      worker.on('message', value => {
        if (slot.stopped || slot.failed || closed) return;
        counts.messages++;
        target.dispatchEvent(new window.MessageEvent('message', { data: value }));
      });
      worker.stdout.resume(); worker.stderr.resume();
    })().catch(() => { if (!slot.worker) slots.delete(slot); fail(slot); });
    return slot;
  }
  function stop(slot) {
    slot.stopped = true;
    slot.stopping ??= (async () => {
      await slot.ready;
      if (!slot.worker || slot.exited) return;
      let failed = false;
      try { await slot.worker.terminate(); } catch { failed = true; }
      await slot.joined;
      if (failed) throw new Error('worker cleanup');
    })();
    slot.stopping.catch(() => {});
    return slot.stopping;
  }
  class DedicatedWorker extends window.EventTarget {
    #slot;
    constructor(url, options) {
      super();
      if (closed || typeof url !== 'string' || options !== undefined || !urls.has(url))
        throw new window.DOMException('Unsupported worker URL or options', 'SecurityError');
      listeners.set(this, {});
      this.#slot = start(urls.get(url), this);
    }
    postMessage(value, options) {
      const slot = this.#slot;
      if (closed || slot.stopped || slot.exited || slot.failed || slot.sent
          || value !== '' || options !== undefined) throw new Error('worker message');
      slot.sent = true;
      slot.ready.then(() => {
        if (!closed && !slot.stopped && !slot.failed && !slot.exited)
          slot.worker.postMessage(value);
      }).catch(() => fail(slot));
    }
    terminate() { stop(this.#slot); }
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
  function createObjectURL(blob) {
    if (closed || !(blob instanceof Blob) || urls.size >= 4)
      throw new Error('unsupported worker blob');
    let size;
    try { size = blobSize.call(blob); } catch { throw new Error('unsupported worker blob'); }
    if (!size || size > 4096) throw new Error('unsupported worker blob');
    const url = createURL(blob);
    urls.set(url, blob); return url;
  }
  function revokeObjectURL(url) {
    if (urls.delete(url)) revokeURL(url);
  }
  const hooks = [[window, 'Worker', DedicatedWorker], [window, 'Blob', Blob],
    [window.URL, 'createObjectURL', createObjectURL],
    [window.URL, 'revokeObjectURL', revokeObjectURL]]
    .map(([target, key, value]) => ({ target, key, value,
      before: Object.getOwnPropertyDescriptor(target, key) }));
  if (hooks.some(h => h.before ? !h.before.configurable : !Object.isExtensible(h.target)))
    throw new Error('worker hooks');
  function restore() {
    let restored = true;
    for (const h of hooks.toReversed()) try {
      if (Object.getOwnPropertyDescriptor(h.target, h.key)?.value !== h.value) continue;
      if (h.before) Object.defineProperty(h.target, h.key, h.before);
      else delete h.target[h.key];
    } catch { restored = false; }
    return restored;
  }
  try {
    for (const h of hooks) Object.defineProperty(h.target, h.key, {
      value: h.value, configurable: true, writable: true, enumerable: h.before?.enumerable ?? true,
    });
  } catch { restore(); throw new Error('worker hooks'); }
  installed.add(window);
  return {
    stats: () => ({ ...counts }),
    close() {
      if (closing) return closing;
      closed = true;
      for (const url of urls.keys()) revokeURL(url);
      urls.clear();
      closing = Promise.allSettled([...slots].map(stop)).then(results => {
        const restored = restore();
        if (!restored || counts.started !== counts.joined
            || results.some(result => result.status === 'rejected'))
          throw new Error('worker cleanup');
        installed.delete(window);
        return { ...counts };
      });
      return closing;
    },
  };
}
