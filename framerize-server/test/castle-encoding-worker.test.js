import assert from "node:assert/strict";
import { getEventListeners } from "node:events";
import { mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setImmediate as tick, setTimeout as delay } from "node:timers/promises";
import { after, test } from "node:test";
import { runEncodingWorker } from "../dist/castle.js";
import { createMinter } from "../dist/castle.js";
import { resolveOptions } from "../dist/castle.js";
import { encode } from "../dist/castle.js";
import { CastleError } from "../dist/castle.js";
import { collectionInput, frame } from "../fixtures/castle/collection-inputs.js";
const worker = new URL("../fixtures/castle/encoding-worker.js", import.meta.url);
test("built-in encoder handles valid, malformed and pre-aborted frames", async () => {
  await privateTest(async (directory) => {
    const controller = new AbortController();
    const input = frame(JSON.stringify(collectionInput()));
    const token = await encode(void 0, [], input, controller.signal, directory);
    assert.ok(token.startsWith("cNz8rPYH|"));
    await assert.rejects(encode(void 0, [], Buffer.alloc(0), controller.signal, directory), failure("encoder"));
    controller.abort();
    await assert.rejects(encode(void 0, [], input, controller.signal, directory), failure("aborted"));
    assert.equal(getEventListeners(controller.signal, "abort").length, 0);
  });
});
test("built-in options reject orphaned executable arguments", () => {
  assert.equal(resolveOptions().encoderPath, void 0);
  assert.throws(() => resolveOptions({ encoderArgs: ["--fixture"] }), failure("configuration"));
});
const failure = (code) => (error) => error instanceof CastleError && error.code === code && !error.message.includes("synthetic-secret");
async function privateTest(run) {
  const directory = await realpath(await mkdtemp(join(tmpdir(), "castle-encoding-")));
  try {
    await run(directory);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}
after(async () => {
  await tick();
  await tick();
  assert.deepEqual(process.getActiveResourcesInfo().filter((name) => /Timeout|TCP|PROCESSWRAP|MESSAGEPORT/i.test(name)), []);
});
async function entered(state) {
  const end = Date.now() + 1500;
  while (Atomics.load(state, 0) !== 1) {
    assert.ok(Date.now() < end, "Encoding fixture did not start");
    await delay(5);
  }
}
for (const mode of [0, 1, 2, 3, 4]) {
  test(`encoding worker mode ${mode} retires before settling`, async () => {
    await privateTest(async (directory) => {
      const shared = new SharedArrayBuffer(12);
      const state = new Int32Array(shared);
      state[2] = mode;
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 3e3);
      const pending = runEncodingWorker(worker, new Uint8Array(shared), directory, controller.signal).finally(() => clearTimeout(timer));
      if (mode === 0) assert.equal(await pending, "cNz8rPYH|QUJD");
      else await assert.rejects(pending, failure("encoder"));
      const count = state[1];
      await delay(25);
      assert.equal(state[1], count);
      assert.equal(getEventListeners(controller.signal, "abort").length, 0);
    });
  });
}
for (const action of ["abort", "close", "timeout"]) {
  test(`minter ${action} stops an active encoding worker before settling`, async () => {
    const shared = new SharedArrayBuffer(12);
    const state = new Int32Array(shared);
    state[2] = 5;
    const controller = new AbortController();
    const stop = (event) => event.stopImmediatePropagation();
    controller.signal.addEventListener("abort", stop);
    const minter = createMinter(resolveOptions({ timeoutMs: 2e3 }), {
      collect: async () => ({ text: "fixture", cuid: null, replayed: 0, traceEvents: 0 }),
      encode: (_path, _args, _frame, signal, directory) => runEncodingWorker(worker, new Uint8Array(shared), directory, signal)
    });
    await checkStop(action, minter, state, controller, stop);
  });
}
async function checkStop(action, minter, state, controller, stop) {
  const pending = minter.mint({ signal: controller.signal });
  const rejected = assert.rejects(pending, failure(action === "timeout" ? "timeout" : "aborted"));
  try {
    await entered(state);
    assert.equal(minter.stats().busy, true);
    if (action === "abort") controller.abort(new Error("synthetic-secret"));
    if (action === "close") await minter.close();
    await rejected;
    const count = state[1];
    await delay(25);
    assert.equal(state[1], count);
    assert.equal(minter.stats().busy, false);
  } finally {
    await minter.close();
    controller.signal.removeEventListener("abort", stop);
  }
  assert.equal(getEventListeners(controller.signal, "abort").length, 0);
}
