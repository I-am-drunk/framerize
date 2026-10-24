import { createHash } from 'node:crypto';
import { Worker } from 'node:worker_threads';
import { classifyReply } from './replies.mjs';

export function createPool(window, hashes) {
  const slots = new Set();
  const counts = { started: 0, joined: 0, errors: 0, messages: 0 };
  const replies = { feature: 0, entropy: 0, timing: 0, error: 0, other: 0 };
  let closed = false, closing;
  function start(blob, target) {
    if (closed || slots.size >= 2) throw new Error('worker limit');
    const slot = { worker: null, stopped: false, failed: false, sent: false };
    slots.add(slot);
    const fail = () => {
      if (slot.failed || slot.stopped) return;
      slot.failed = true; counts.errors++;
      target.dispatchEvent(new window.ErrorEvent('error', { message: 'worker failed' }));
    };
    slot.ready = (async () => {
      const bytes = Buffer.from(await blob.arrayBuffer());
      if (!hashes.has(createHash('sha256').update(bytes).digest('hex')))
        throw new Error('worker source');
      if (slot.stopped || closed) return;
      slot.worker = new Worker(new URL('./runtime.mjs', import.meta.url), {
        workerData: { source: bytes.toString('utf8'), userAgent: window.navigator.userAgent },
        resourceLimits: { maxOldGenerationSizeMb: 16, maxYoungGenerationSizeMb: 4, stackSizeMb: 2 },
        stdout: true, stderr: true,
      });
      counts.started++;
      slot.worker.stdout.resume(); slot.worker.stderr.resume();
      slot.worker.on('error', fail);
      slot.worker.on('exit', () => { counts.joined++; if (!slot.stopped) fail(); });
      slot.worker.on('message', value => {
        if (slot.stopped) return;
        counts.messages++;
        replies[classifyReply(value)]++;
        target.dispatchEvent(new window.MessageEvent('message', { data: value }));
      });
    })().catch(fail);
    slot.stop = () => {
      slot.stopped = true;
      slot.stopping ??= slot.ready.then(() => slot.worker?.terminate());
      return slot.stopping;
    };
    slot.send = value => {
      if (slot.stopped || slot.sent || value !== '') throw new Error('worker message');
      slot.sent = true;
      slot.ready.then(() => {
        if (!slot.stopped && !slot.failed) slot.worker.postMessage(value);
      }).catch(fail);
    };
    return slot;
  }
  return {
    start,
    stats: () => ({ ...counts }),
    replyStats: () => ({ ...replies }),
    close() {
      closed = true;
      closing ??= Promise.all([...slots].map(slot => slot.stop())).then(() => {
        if (counts.started !== counts.joined) throw new Error('worker cleanup');
        return { ...counts };
      });
      return closing;
    },
  };
}
