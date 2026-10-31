import { Blob } from 'node:buffer';
import { createHash, randomUUID } from 'node:crypto';
import { MessageChannel, Worker } from 'node:worker_threads';

export function installSharedWorkers(window, expectedScriptSha256) {
  if (window.location.origin !== 'https://framer.com'
      || typeof window.navigator.userAgent !== 'string'
      || window.navigator.userAgent.length === 0
      || !/^[a-f0-9]{64}$/.test(expectedScriptSha256)) throw new Error('worker scope');
  const urls = new Map(), slots = new Map();
  const counts = { started: 0, joined: 0, errors: 0, connections: 0, messages: 0, sdk_replies: 0 };
  let closed = false, closing;
  function fail(slot) {
    if (slot.failed || closed) return;
    slot.failed = true; counts.errors++;
    for (const client of slot.clients)
      client.dispatchEvent(new window.ErrorEvent('error', { message: 'shared worker failed' }));
  }
  function start(url) {
    const blob = urls.get(url);
    if (closed || !blob) throw new window.DOMException('Unregistered worker URL', 'SecurityError');
    if (slots.size >= 1) throw new window.DOMException('Worker limit', 'QuotaExceededError');
    const slot = { clients: [], ports: [], worker: null, failed: false };
    slots.set(url, slot);
    slot.ready = (async () => {
      const bytes = Buffer.from(await blob.arrayBuffer());
      if (createHash('sha256').update(bytes).digest('hex') !== expectedScriptSha256)
        throw new Error('worker source');
      if (closed) return;
      slot.worker = new Worker(new URL('./shared_runtime.mjs', import.meta.url), {
        workerData: { source: bytes.toString('utf8'), userAgent: window.navigator.userAgent },
        resourceLimits: { maxOldGenerationSizeMb: 16, maxYoungGenerationSizeMb: 4, stackSizeMb: 2 },
        stdout: true, stderr: true,
      });
      counts.started++;
      slot.joined = new Promise(resolve => slot.worker.once('exit', () => {
        slot.exited = true; counts.joined++;
        resolve(); if (!closed) fail(slot);
      }));
      slot.worker.stdout.resume(); slot.worker.stderr.resume();
      slot.worker.on('error', () => fail(slot));
    })().catch(() => fail(slot));
    return slot;
  }
  class SharedWorker extends window.EventTarget {
    #onerror = null;
    constructor(url, options) {
      super();
      if (closed || typeof url !== 'string' || options !== undefined || !urls.has(url))
        throw new window.DOMException('Unsupported worker URL or options', 'SecurityError');
      if (counts.connections >= 4) throw new window.DOMException('Port limit', 'QuotaExceededError');
      const slot = slots.get(url) ?? start(url);
      const { port1, port2 } = new MessageChannel();
      port1.on('message', value => {
        counts.messages++;
        if (Array.isArray(value) && value.length === 13 && value[0] === 2
            && value[1] === false && value[2] === window.navigator.userAgent && value[12] === 'NA'
            && Array.from({length:9}, (_,i) => value[i+3]).every(item => item === undefined))
          counts.sdk_replies++;
      });
      Object.defineProperty(this, 'port', { value: port1, enumerable: true });
      slot.clients.push(this); slot.ports.push(port1, port2); counts.connections++;
      slot.ready.then(() => {
        if (!closed && !slot.failed) slot.worker.postMessage({ port: port2 }, [port2]);
      }).catch(() => fail(slot));
    }
    get onerror() { return this.#onerror; }
    set onerror(value) {
      if (this.#onerror) this.removeEventListener('error', this.#onerror);
      this.#onerror = typeof value === 'function' ? value : null;
      if (this.#onerror) this.addEventListener('error', this.#onerror);
    }
  }
  Object.defineProperty(window, 'Blob', { value: Blob, configurable: true, writable: true });
  window.URL.createObjectURL = blob => {
    if (closed || !(blob instanceof Blob) || blob.type !== 'application/javascript'
        || blob.size > 4096 || urls.size >= 4) throw new Error('unsupported worker blob');
    const url = `blob:${window.location.origin}/${randomUUID()}`;
    urls.set(url, blob); return url;
  };
  window.URL.revokeObjectURL = url => { urls.delete(url); };
  Object.defineProperty(window, 'SharedWorker', { value: SharedWorker, configurable: true });
  return {
    SharedWorker,
    stats: () => ({ ...counts }),
    close() {
      if (closing) return closing;
      closed = true; urls.clear();
      for (const slot of slots.values()) for (const port of slot.ports) port.close();
      closing = (async () => {
        const prepared = await Promise.allSettled([...slots.values()].map(slot => slot.ready));
        const stopped = await Promise.allSettled([...slots.values()].filter(slot => slot.worker)
          .map(async slot => {
            let failed = false;
            try { if (!slot.exited) await slot.worker.terminate(); }
            catch { failed = true; }
            // A stop failure is retained, but cannot release a live worker.
            await slot.joined;
            if (failed) throw new Error('worker cleanup');
          }));
        if ([...prepared, ...stopped].some(result => result.status === 'rejected')
            || counts.joined !== counts.started) throw new Error('worker cleanup');
        return { ...counts };
      })();
      return closing;
    },
  };
}
