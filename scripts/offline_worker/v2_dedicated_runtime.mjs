import { Buffer } from 'node:buffer';
import { createHash } from 'node:crypto';
import { performance } from 'node:perf_hooks';
import { parentPort, workerData } from 'node:worker_threads';
import vm from 'node:vm';

const source = workerData?.source, sha256 = workerData?.sha256;
if (!parentPort || typeof source !== 'string' || typeof sha256 !== 'string'
    || Object.keys(workerData).sort().join(',') !== 'sha256,source'
    || !/^[a-f0-9]{64}$/.test(sha256)) throw new Error('worker input');
const bytes = Buffer.from(source, 'utf8');
if (!bytes.length || bytes.length > 4096 || bytes.toString('utf8') !== source
    || createHash('sha256').update(bytes).digest('hex') !== sha256)
  throw new Error('worker source');

// Actual Node capabilities, not a synthesized browser identity. The caller
// still owns process containment; vm is not a security boundary.
const navigator = globalThis.navigator;
if (!navigator || typeof navigator !== 'object') throw new Error('worker navigator');
const context = vm.createContext({ navigator, performance,
  postMessage: value => parentPort.postMessage(value),
}, { codeGeneration: { strings: false, wasm: false } });
context.self = context;
try {
  new vm.Script(source, { filename: 'v2-dedicated-worker.js' })
    .runInContext(context, { timeout: 1000 });
} catch { throw new Error('worker execution'); }
if (typeof context.onmessage !== 'function') throw new Error('worker message handler');
const dispatch = new vm.Script('self.onmessage({ data: "" })', {
  filename: 'v2-dedicated-message.js',
});
let received = false;
parentPort.on('message', value => {
  if (received || value !== '') throw new Error('worker message');
  received = true;
  try {
    Promise.resolve(dispatch.runInContext(context, { timeout: 1000 }))
      .catch(() => { throw new Error('worker message failed'); });
  } catch { throw new Error('worker message failed'); }
});
