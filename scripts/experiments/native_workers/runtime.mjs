import { parentPort, workerData } from 'node:worker_threads';
import { performance } from 'node:perf_hooks';
import vm from 'node:vm';

if (!parentPort || typeof workerData?.source !== 'string'
    || workerData.source.length > 4096 || workerData.userAgent !== 'Framerize/0.1')
  throw new Error('worker input');
const context = vm.createContext({
  navigator: Object.freeze({ userAgent: workerData.userAgent }),
  performance,
  postMessage: value => parentPort.postMessage(value),
}, { codeGeneration: { strings: false, wasm: false } });
context.self = context;
new vm.Script(workerData.source, { filename: 'sdk-dedicated-worker.js' })
  .runInContext(context, { timeout: 1000 });
if (typeof context.onmessage !== 'function') throw new Error('worker message handler');
parentPort.on('message', value => {
  Promise.resolve(context.onmessage({ data: value }))
    .catch(() => { throw new Error('worker message failed'); });
});
