import { parentPort, workerData } from "node:worker_threads";
const { mode, shared } = workerData;
const state = new Int32Array(shared);
Atomics.store(state, 0, 1);
if (mode === "success") parentPort?.postMessage({ result: {
  text: "collection",
  cuid: null,
  replayed: 0,
  traceEvents: 0
} });
if (mode === "malformed") parentPort?.postMessage({ result: { text: 12 } });
if (mode === "failure") throw new Error("synthetic-secret");
if (mode === "output") process.stdout.write("x".repeat(128 * 1024));
if (mode === "error-output") process.stderr.write("x".repeat(128 * 1024));
setInterval(() => Atomics.add(state, 1, 1), 10);
