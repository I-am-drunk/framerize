import { parentPort, workerData } from 'node:worker_threads';
import vm from 'node:vm';
// The parent admits one exact script digest and supplies only its native UA.
if (!parentPort || typeof workerData?.source !== 'string'
    || workerData.userAgent !== 'Framerize/0.1') throw new Error('worker input');
const navigator = Object.freeze({ userAgent: workerData.userAgent });
const context = vm.createContext({ navigator }, { codeGeneration: { strings: false, wasm: false } });
context.self = context;
new vm.Script(workerData.source, { filename: 'sdk-shared-worker.js' })
  .runInContext(context, { timeout: 1000 });
if (typeof context.onconnect !== 'function') throw new Error('worker connection handler');
parentPort.on('message', message => {
  if (!message?.port || Object.keys(message).join(',') !== 'port')
    throw new Error('worker connection');
  Promise.resolve(context.onconnect({ ports: [message.port] }))
    .catch(() => { throw new Error('worker connection failed'); });
});
