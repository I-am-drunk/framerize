import assert from "node:assert/strict";
import { getEventListeners } from "node:events";
import { mkdtemp, readFile, rm, realpath, writeFile } from "node:fs/promises";
import { Worker } from "node:worker_threads";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { setImmediate as tick, setTimeout as delay } from "node:timers/promises";
import { after, test } from "node:test";
import { encode } from "../dist/castle.js";
import { runCollectionWorker } from "../dist/castle.js";
import { CastleError } from "../dist/castle.js";
import { privateEnvironment } from "../dist/castle.js";
const encoder = fileURLToPath(new URL("../fixtures/castle/encoder.js", import.meta.url));
const worker = new URL("../fixtures/castle/worker.js", import.meta.url);
const failure = (code) => (error) => error instanceof CastleError && error.code === code && !error.message.includes("synthetic-secret");
async function privateTest(run) {
  const directory = await realpath(await mkdtemp(join(tmpdir(), "castle-operations-")));
  try {
    await run(directory);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}
async function until(predicate) {
  const end = Date.now() + 5e3;
  while (!await predicate()) {
    if (Date.now() >= end) throw new Error("Synthetic fixture did not start");
    await delay(5);
  }
}
for (const guard of [true, false]) {
  test(`worker guard ${guard ? "blocks ambient operations" : "negative control is detected"}`, async () => {
    await privateTest(async (directory) => {
      const modulePath = join(directory, "approved.mjs");
      const moduleSource = 'export const marker = "synthetic-module";';
      const diskSource = 'throw new Error("Unapproved disk source executed");';
      await writeFile(modulePath, diskSource);
      const child = new Worker(new URL("../fixtures/castle/guard.js", import.meta.url), {
        workerData: { directory, modulePath, moduleSource, guard },
        execArgv: [],
        env: privateEnvironment(directory)
      });
      let timer;
      try {
        const pending = new Promise((resolve, reject) => {
          child.once("message", resolve);
          child.once("error", reject);
          child.once("exit", () => reject(new Error("Guard fixture exited before reporting")));
          timer = setTimeout(() => reject(new Error("Guard fixture timed out")), 5e3);
        });
        if (guard) assert.deepEqual(await pending, { checks: 20, imported: true });
        else await assert.rejects(pending, /Underlying operation reached/);
      } finally {
        clearTimeout(timer);
        await child.terminate();
      }
      assert.equal(await readFile(modulePath, "utf8"), diskSource);
    });
  });
}
after(async () => {
  await tick();
  await tick();
  assert.deepEqual(process.getActiveResourcesInfo().filter((name) => /Timeout|TCP|PROCESSWRAP|MESSAGEPORT/i.test(name)), []);
});
test("encoder receives exact bytes in a private environment and directory", async () => {
  await privateTest(async (directory) => {
    const keys = ["CASTLE_PRIVATE_CANARY", "NODE_USE_ENV_PROXY", "HTTPS_PROXY"];
    const previous = keys.map((key) => process.env[key]);
    for (const key of keys) process.env[key] = "synthetic-secret";
    const controller = new AbortController();
    try {
      assert.equal(await encode(
        process.execPath,
        [encoder, "success"],
        Buffer.from("fixture"),
        controller.signal,
        directory
      ), Buffer.from("fixture").toString("hex"));
    } finally {
      keys.forEach((key, index) => {
        if (previous[index] === void 0) delete process.env[key];
        else process.env[key] = previous[index];
      });
    }
    assert.equal(getEventListeners(controller.signal, "abort").length, 0);
  });
});
for (const mode of ["invalid", "empty", "failure", "stdout", "stderr"]) {
  test(`encoder rejects ${mode} safely and reaps its process`, async () => {
    await privateTest(async (directory) => {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 5e3);
      try {
        await assert.rejects(encode(
          process.execPath,
          [encoder, mode],
          Buffer.from("fixture"),
          controller.signal,
          directory
        ), failure("encoder"));
      } finally {
        clearTimeout(timer);
      }
      assert.equal(getEventListeners(controller.signal, "abort").length, 0);
      if (mode === "stdout" || mode === "stderr") {
        const pid = Number(await readFile(join(directory, "started"), "utf8"));
        assert.throws(() => process.kill(pid, 0), { code: "ESRCH" });
      }
    });
  });
}
test("encoder abort cannot be suppressed and waits for process exit", async () => {
  await privateTest(async (directory) => {
    const controller = new AbortController();
    const stop = (event) => event.stopImmediatePropagation();
    controller.signal.addEventListener("abort", stop);
    const pending = encode(process.execPath, [encoder, "hang"], Buffer.from("fixture"), controller.signal, directory);
    const rejected = assert.rejects(pending, failure("aborted"));
    let pid = 0;
    try {
      await until(async () => {
        try {
          pid = Number(await readFile(join(directory, "started"), "utf8"));
          return pid > 0;
        } catch {
          return false;
        }
      });
    } finally {
      controller.abort();
    }
    await rejected;
    controller.signal.removeEventListener("abort", stop);
    assert.equal(getEventListeners(controller.signal, "abort").length, 0);
    assert.throws(() => process.kill(pid, 0), { code: "ESRCH" });
  });
});
test("encoder pre-abort and unavailable executable have fixed errors", async () => {
  await privateTest(async (directory) => {
    const controller = new AbortController();
    controller.abort(new Error("synthetic-secret"));
    await assert.rejects(encode("missing", [], Buffer.alloc(0), controller.signal, directory), failure("aborted"));
    await assert.rejects(encode(
      join(directory, "missing"),
      [],
      Buffer.alloc(0),
      new AbortController().signal,
      directory
    ), failure("encoder"));
  });
});
for (const mode of ["success", "malformed", "failure", "output", "error-output", "timeout"]) {
  test(`worker ${mode} settles only after termination`, async () => {
    await privateTest(async (directory) => {
      const shared = new SharedArrayBuffer(8);
      const state = new Int32Array(shared);
      const controller = new AbortController();
      const pending = runCollectionWorker(worker, { mode, shared }, directory, controller.signal, 1e3);
      if (mode === "success") assert.deepEqual(
        await pending,
        { text: "collection", cuid: null, replayed: 0, traceEvents: 0 }
      );
      else await assert.rejects(pending, failure(mode === "timeout" ? "timeout" : "collection"));
      const count = Atomics.load(state, 1);
      await delay(25);
      assert.equal(Atomics.load(state, 1), count);
      assert.equal(getEventListeners(controller.signal, "abort").length, 0);
    });
  });
}
test("worker cancellation survives stopped event propagation", async () => {
  await privateTest(async (directory) => {
    const shared = new SharedArrayBuffer(8);
    const state = new Int32Array(shared);
    const controller = new AbortController();
    const stop = (event) => event.stopImmediatePropagation();
    controller.signal.addEventListener("abort", stop);
    const pending = runCollectionWorker(worker, { mode: "hang", shared }, directory, controller.signal, 5e3);
    const rejected = assert.rejects(pending, failure("aborted"));
    try {
      await until(() => Atomics.load(state, 0) === 1);
    } finally {
      controller.abort();
    }
    await rejected;
    controller.signal.removeEventListener("abort", stop);
    assert.equal(getEventListeners(controller.signal, "abort").length, 0);
    const count = Atomics.load(state, 1);
    await delay(25);
    assert.equal(Atomics.load(state, 1), count);
  });
});
