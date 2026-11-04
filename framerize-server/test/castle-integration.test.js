import assert from "node:assert/strict";
import { getEventListeners } from "node:events";
import { setImmediate as tick, setTimeout as delay } from "node:timers/promises";
import { after, test } from "node:test";
import { createCastleMinter } from "../dist/castle.js";
after(async () => {
  await tick();
  await tick();
  assert.deepEqual(process.getActiveResourcesInfo().filter((name) => /Timeout|TCP|PROCESSWRAP|MESSAGEPORT/i.test(name)), []);
});
for (const cuid of [void 0, "synthetic-integration"]) {
  test(`typed collector encodes with ${cuid ? "explicit" : "absent"} CUID and cleans up`, { timeout: 15e3 }, async (context) => {
    const listeners = getEventListeners(context.signal, "abort").length;
    const minter = createCastleMinter();
    try {
      const result = await minter.mint({ ...cuid === void 0 ? {} : { cuid }, signal: context.signal });
      assert.match(result.token, /^cNz8rPYH\|[A-Za-z0-9+/]+={0,2}$/);
      assert.ok(result.token.length <= 16384);
      assert.equal(result.cuid, cuid ?? null);
      assert.ok(result.collectedLength > 0);
      assert.equal(result.traceEvents, 198);
      assert.ok(result.replayed > 0);
      assert.equal(result.encoding, "wire-token");
      assert.equal(minter.stats().mints, 1);
      assert.equal(minter.stats().busy, false);
      assert.equal(getEventListeners(context.signal, "abort").length, listeners);
      context.diagnostic(JSON.stringify({
        collectedLength: result.collectedLength,
        traceEvents: result.traceEvents,
        replayed: result.replayed
      }));
    } finally {
      await minter.close();
    }
  });
}
for (const action of ["abort", "close"]) {
  test(`typed collector ${action} stops an active worker`, { timeout: 1e4 }, async () => {
    const resources = () => process.getActiveResourcesInfo().filter((name) => /MESSAGEPORT/i.test(name)).length;
    const baseline = resources();
    const controller = new AbortController();
    const minter = createCastleMinter();
    const pending = minter.mint({ signal: controller.signal });
    const rejection = assert.rejects(pending, { code: "aborted" });
    try {
      const deadline = Date.now() + 3e3;
      while (resources() <= baseline) {
        assert.ok(Date.now() < deadline, "Collection worker did not start");
        await delay(5);
      }
      assert.equal(minter.stats().busy, true);
      if (action === "abort") controller.abort(new Error("synthetic-secret"));
      else await Promise.all([minter.close(), minter.close()]);
      await rejection;
      assert.equal(minter.stats().busy, false);
      assert.equal(minter.stats().failures, 1);
      assert.equal(getEventListeners(controller.signal, "abort").length, 0);
    } finally {
      await minter.close();
    }
  });
}
