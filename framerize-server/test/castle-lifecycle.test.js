import assert from "node:assert/strict";
import { getEventListeners } from "node:events";
import { access, realpath, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { setImmediate as tick, setTimeout as delay } from "node:timers/promises";
import { after, test } from "node:test";
import { createMinter } from "../dist/castle.js";
import { CastleError } from "../dist/castle.js";
import { resolveOptions } from "../dist/castle.js";
const collection = { text: "synthetic-\xE9", cuid: "fixture", replayed: 2, traceEvents: 3 };
const options = () => resolveOptions({ encoderPath: process.execPath, timeoutMs: 2e3 });
after(async () => {
  await tick();
  await tick();
  const resources = process.getActiveResourcesInfo().filter((name) => /Timeout|TCP|PIPECONNECTWRAP|PROCESSWRAP|MESSAGEPORT/i.test(name));
  assert.deepEqual(resources, []);
});
const operations = () => ({ collect: async () => collection, encode: async () => "token" });
const failure = (code) => (error) => error instanceof CastleError && error.code === code && error.message === new CastleError(code).message;
function deferred() {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
test("frames UTF-8 collection and cleans private storage before returning", async () => {
  let directory = "";
  const minter = createMinter(options(), {
    collect: async (input, signal, budget) => {
      directory = input.directory;
      assert.equal(directory, await realpath(directory));
      if (process.platform !== "win32") assert.equal((await stat(directory)).mode & 511, 448);
      assert.equal(input.cuid, "fixture");
      assert.equal(budget, 8e3);
      assert.equal(signal.aborted, false);
      await writeFile(join(directory, "synthetic-state"), "fixture");
      return collection;
    },
    encode: async (path, args, frame, signal, storage) => {
      assert.equal(path, process.execPath);
      assert.deepEqual(args, []);
      assert.equal(storage, directory);
      assert.equal(signal.aborted, false);
      assert.equal(frame.subarray(0, 4).toString(), "FCLT");
      assert.deepEqual([...frame.subarray(4, 8)], [1, 0, 0, 0]);
      assert.equal(frame.readUInt32BE(8), Buffer.byteLength(collection.text));
      assert.equal(frame.subarray(12).toString(), collection.text);
      return "token";
    }
  });
  const result = await minter.mint({ cuid: "fixture" });
  assert.deepEqual(result, {
    token: "token",
    cuid: "fixture",
    tokenLength: 5,
    collectedLength: collection.text.length,
    replayed: 2,
    traceEvents: 3,
    encoding: "wire-token"
  });
  assert.ok(Object.isFrozen(result));
  assert.ok(Object.isFrozen(minter.stats()));
  assert.deepEqual(minter.stats(), { mints: 1, failures: 0, busy: false, traceEvents: 3 });
  await assert.rejects(access(directory), { code: "ENOENT" });
  await minter.close();
});
test("rejects pre-abort without invoking adapters or retaining listeners", async () => {
  const controller = new AbortController();
  controller.abort(new Error("synthetic-secret"));
  let calls = 0;
  const minter = createMinter(options(), { ...operations(), collect: async () => {
    calls += 1;
    return collection;
  } });
  await assert.rejects(minter.mint({ signal: controller.signal }), failure("aborted"));
  assert.equal(calls, 0);
  assert.equal(getEventListeners(controller.signal, "abort").length, 0);
  assert.deepEqual(minter.stats(), { mints: 0, failures: 1, busy: false, traceEvents: 0 });
  await minter.close();
});
for (const stage of ["collect", "encode"]) {
  test(`sanitizes ${stage} rejection and removes its temporary directory`, async () => {
    let directory = "";
    const adapters = operations();
    const minter = createMinter(options(), {
      collect: async (input) => {
        directory = input.directory;
        if (stage === "collect") throw new Error("synthetic-secret");
        return collection;
      },
      encode: stage === "encode" ? () => {
        throw new Error("synthetic-secret");
      } : adapters.encode
    });
    await assert.rejects(minter.mint(), failure(stage === "collect" ? "collection" : "encoder"));
    await assert.rejects(access(directory), { code: "ENOENT" });
    assert.equal(minter.stats().busy, false);
    assert.equal(minter.stats().failures, 1);
    await minter.close();
  });
}
for (const stage of ["collect", "encode"]) {
  test(`abort waits for ${stage} cleanup and preserves the first cancellation`, async () => {
    const entered = deferred(), release = deferred(), stopped = deferred();
    const controller = new AbortController();
    let signal;
    let directory = "";
    async function hold(input) {
      signal = input;
      input.addEventListener("abort", stopped.resolve, { once: true });
      entered.resolve();
      await release.promise;
      input.removeEventListener("abort", stopped.resolve);
    }
    const minter = createMinter(resolveOptions({ encoderPath: process.execPath, timeoutMs: 200 }), {
      collect: async (input, inputSignal) => {
        directory = input.directory;
        if (stage === "collect") await hold(inputSignal);
        return collection;
      },
      encode: async (_path, _args, _frame, inputSignal) => {
        await hold(inputSignal);
        return "token";
      }
    });
    const mint = minter.mint({ signal: controller.signal });
    const rejection = assert.rejects(mint, failure("aborted"));
    try {
      await entered.promise;
      controller.abort(new Error("synthetic-secret"));
      await stopped.promise;
      assert.equal(signal?.aborted, true);
      await delay(250);
      assert.equal(minter.stats().busy, true);
      await assert.rejects(minter.mint(), failure("busy"));
    } finally {
      release.resolve();
    }
    await rejection;
    assert.equal(minter.stats().busy, false);
    assert.equal(getEventListeners(controller.signal, "abort").length, 0);
    await assert.rejects(access(directory), { code: "ENOENT" });
    assert.deepEqual(minter.stats(), { mints: 0, failures: 2, busy: false, traceEvents: 0 });
    await minter.close();
  });
}
test("close aborts active work and awaits cleanup exactly once", async () => {
  const entered = deferred(), release = deferred();
  let signal;
  const minter = createMinter(options(), { ...operations(), collect: async (_input, inputSignal) => {
    signal = inputSignal;
    entered.resolve();
    await release.promise;
    return collection;
  } });
  const mint = minter.mint();
  const rejection = assert.rejects(mint, failure("aborted"));
  await entered.promise;
  let closed = false;
  const first = minter.close().then(() => {
    closed = true;
  });
  const second = minter.close();
  await Promise.resolve();
  assert.equal(signal?.aborted, true);
  assert.equal(closed, false);
  await assert.rejects(minter.mint(), failure("closed"));
  release.resolve();
  await Promise.all([first, second, rejection]);
  assert.equal(minter.stats().busy, false);
});
test("overall timeout aborts work before rejecting", async () => {
  let aborted = false;
  const minter = createMinter(resolveOptions({ encoderPath: process.execPath, timeoutMs: 40 }), {
    ...operations(),
    collect: async (_input, signal) => {
      await new Promise((resolve) => signal.addEventListener("abort", () => {
        aborted = true;
        resolve();
      }, { once: true }));
      return collection;
    }
  });
  await assert.rejects(minter.mint(), failure("timeout"));
  assert.equal(aborted, true);
  assert.equal(minter.stats().busy, false);
  await minter.close();
});
test("another listener cannot suppress cancellation", async () => {
  const controller = new AbortController();
  const stop = (event) => event.stopImmediatePropagation();
  controller.signal.addEventListener("abort", stop);
  let cancelled = false;
  const minter = createMinter(options(), { ...operations(), collect: async (_input, signal) => {
    controller.abort();
    cancelled = signal.aborted;
    return collection;
  } });
  await assert.rejects(minter.mint({ signal: controller.signal }), failure("aborted"));
  assert.equal(cancelled, true);
  controller.signal.removeEventListener("abort", stop);
  assert.equal(getEventListeners(controller.signal, "abort").length, 0);
  await minter.close();
});
test("request validation uses fixed errors for invalid or active inputs", async () => {
  const minter = createMinter(options(), operations());
  const throwing = new Proxy({}, { get() {
    throw new Error("synthetic-secret");
  } });
  const invalid = [
    null,
    4,
    "request",
    [],
    { cuid: "" },
    { cuid: "bad value" },
    { signal: {} },
    throwing,
    { signal: Object.create(AbortSignal.prototype) }
  ];
  for (const request of invalid) {
    await assert.rejects(minter.mint(request), failure("configuration"));
  }
  assert.equal(minter.stats().failures, invalid.length);
  assert.equal(minter.stats().busy, false);
  await minter.close();
});
test("options reject invalid durations and do not repeat caller getters", () => {
  for (const timeoutMs of [0, -1, NaN, Infinity, 2 ** 31, 0.5, null, "100"]) {
    const input2 = { encoderPath: process.execPath, timeoutMs };
    assert.throws(() => resolveOptions(input2), failure("configuration"));
  }
  let reads = 0;
  const input = { get encoderPath() {
    reads += 1;
    return reads === 1 ? process.execPath : "invalid-relative-path";
  } };
  assert.equal(resolveOptions(input).encoderPath, process.execPath);
  assert.equal(reads, 1);
  const throwing = new Proxy({}, { get() {
    throw new Error("synthetic-secret");
  } });
  assert.throws(() => resolveOptions(throwing), failure("configuration"));
  const args = ["--fixture"];
  const resolved = resolveOptions({ encoderPath: process.execPath, encoderArgs: args });
  args.push("new-value");
  assert.deepEqual(resolved.encoderArgs, ["--fixture"]);
  assert.ok(Object.isFrozen(resolved));
  assert.ok(Object.isFrozen(resolved.encoderArgs));
});
test("successful calls use separate storage and remove caller listeners", async () => {
  const directories = [];
  const controller = new AbortController();
  const minter = createMinter(options(), { ...operations(), collect: async (input) => {
    directories.push(input.directory);
    return collection;
  } });
  for (let index = 0; index < 2; index += 1) {
    await minter.mint({ signal: controller.signal });
    assert.equal(getEventListeners(controller.signal, "abort").length, 0);
  }
  assert.equal(new Set(directories).size, 2);
  for (const directory of directories) await assert.rejects(access(directory), { code: "ENOENT" });
  assert.deepEqual(minter.stats(), { mints: 2, failures: 0, busy: false, traceEvents: 3 });
  await minter.close();
});
