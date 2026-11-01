import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../camoufox_host.py", import.meta.url), "utf8");
function script(name) {
  const match = source.match(new RegExp("^" + name + ' = """([\\s\\S]*?)"""', "m"));
  assert.ok(match, name + " must exist in the sidecar");
  return match[1].replace("%s", "http://fixture.invalid");
}

function harness(fetch) {
  const timers = new Map();
  let nextTimer = 0;
  const context = vm.createContext({
    window: {}, AbortController, fetch,
    localStorage: { setItem() {} }, document: { cookie: "" },
    setTimeout(fn) { const id = ++nextTimer; timers.set(id, fn); return id; },
    clearTimeout(id) { timers.delete(id); },
  });
  return {
    context, timers,
    start: vm.runInContext(script("AI_START_JS"), context),
    poll: vm.runInContext(script("AI_POLL_JS"), context),
    cancel: vm.runInContext(script("AI_CANCEL_JS"), context),
  };
}

const response = () => ({
  status: 200, headers: { get: () => null },
  text: async () => "fixture".repeat(10000),
});
const flush = () => new Promise(resolve => setImmediate(resolve));
const args = ["/response", {}, "fixture-token", null, "job-1", null];

test("consuming a completed response removes its buffer and timer", async () => {
  const h = harness(async () => response());
  await h.start(structuredClone(args));
  await flush();
  assert.equal(h.poll("job-1").status, 200);
  assert.equal(Object.keys(h.context.window.__aiJobs).length, 0);
  assert.equal(h.timers.size, 0);
});

test("cancel aborts the fetch and late completion cannot recreate the job", async () => {
  let complete, signal;
  const h = harness((url, options) => {
    signal = options.signal;
    return new Promise(resolve => { complete = resolve; });
  });
  await h.start(structuredClone(args));
  assert.equal(h.cancel("job-1"), true);
  assert.equal(signal.aborted, true);
  complete(response());
  await flush();
  assert.equal(Object.keys(h.context.window.__aiJobs).length, 0);
  assert.equal(h.timers.size, 0);
});

test("expiry releases a job if the host disappears", async () => {
  let complete, signal;
  const h = harness((url, options) => {
    signal = options.signal;
    return new Promise(resolve => { complete = resolve; });
  });
  await h.start(structuredClone(args));
  const expire = [...h.timers.values()][0];
  expire();
  assert.equal(signal.aborted, true);
  complete(response());
  await flush();
  assert.equal(Object.keys(h.context.window.__aiJobs).length, 0);
});
