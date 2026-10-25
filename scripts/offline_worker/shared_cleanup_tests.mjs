// Inert worker ownership checks. No SDK, browser or account access.
import test from 'node:test';
import assert from 'node:assert/strict';
import { Blob } from 'node:buffer';
import { createHash, randomUUID } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const source = '// inert shared worker';
const turn = () => new Promise(resolve => setImmediate(resolve));
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function observe(promise) {
  const state = { settled: false };
  state.done = promise.then(value => Object.assign(state, { settled: true, value }),
    error => Object.assign(state, { settled: true, error }));
  return state;
}
async function fixture(t) {
  const threads = [], ports = [], reading = deferred();
  class Thread extends EventEmitter {
    constructor() {
      super(); threads.push(this); this.stop = deferred(); this.calls = 0;
      this.stdout = this.stderr = { resume() {} };
    }
    postMessage() {}
    terminate() {
      this.calls++;
      if (this.throws) throw new Error('private fixture failure');
      return this.stop.promise;
    }
    exit() { if (!this.exited) { this.exited = true; this.emit('exit', 0); } }
  }
  class Port extends EventEmitter {
    constructor() { super(); ports.push(this); }
    close() { this.closed = true; }
  }
  class Channel { port1 = new Port(); port2 = new Port(); }
  class InputBlob extends Blob { arrayBuffer() { return reading.promise; } }
  const url = new URL('./shared_workers.mjs', import.meta.url);
  const context = vm.createContext({ Buffer, URL });
  const module = new vm.SourceTextModule(await readFile(url, 'utf8'), {
    context, initializeImportMeta(meta) { meta.url = url.href; },
  });
  const imports = { 'node:buffer': { Blob: InputBlob },
    'node:crypto': { createHash, randomUUID },
    'node:worker_threads': { Worker: Thread, MessageChannel: Channel } };
  await module.link(name => {
    assert.ok(Object.hasOwn(imports, name), 'unexpected import');
    const entries = Object.entries(imports[name]);
    return new vm.SyntheticModule(entries.map(([key]) => key), function () {
      for (const [key, value] of entries) this.setExport(key, value);
    }, { context });
  });
  await module.evaluate();
  class DataEvent extends Event {
    constructor(type, options) { super(type); Object.assign(this, options); }
  }
  const window = { location: { origin: 'https://framer.com' },
    navigator: { userAgent: 'Framerize/0.1' }, URL: {},
    EventTarget, ErrorEvent: DataEvent, DOMException };
  const bridge = module.namespace.installSharedWorkers(window,
    createHash('sha256').update(source).digest('hex'));
  const blob = new window.Blob([source], { type: 'application/javascript' });
  const h = { bridge, threads, ports, failure: false,
    load: () => reading.resolve(Buffer.from(source)),
    start: () => new window.SharedWorker(window.URL.createObjectURL(blob)),
  };
  t.after(async () => {
    const closing = observe(bridge.close()); h.load(); await turn();
    for (const thread of threads) { thread.stop.resolve(0); thread.exit(); }
    await closing.done;
    assert.equal(Boolean(closing.error), h.failure);
    assert.ok(ports.every(port => port.closed));
  });
  return h;
}

test('close retains ownership until exit after stop settles', { timeout: 3000 }, async t => {
  for (const outcome of ['resolve', 'reject', 'throw']) {
    await t.test(outcome, async t => {
      const h = await fixture(t); h.start(); h.load(); await turn();
      const worker = h.threads[0]; h.failure = outcome !== 'resolve';
      worker.throws = outcome === 'throw';
      const closing = observe(h.bridge.close()); await turn();
      if (outcome === 'reject') worker.stop.reject(new Error('private fixture failure'));
      if (outcome === 'resolve') worker.stop.resolve(0);
      await turn();
      assert.equal(closing.settled, false, 'stop settlement is not worker exit');
      assert.equal(h.bridge.stats().joined, 0);
      worker.exit(); await closing.done;
      assert.equal(Boolean(closing.error), h.failure);
      if (h.failure) assert.equal(closing.error.message, 'worker cleanup');
      assert.equal(h.bridge.stats().joined, 1);
      assert.equal(worker.calls, 1);
    });
  }
});

test('exit retains a pending failed stop and close is idempotent', { timeout: 3000 }, async t => {
  const h = await fixture(t); h.start(); h.load(); await turn();
  const worker = h.threads[0], first = h.bridge.close(), closing = observe(first);
  assert.equal(h.bridge.close(), first); await turn();
  worker.exit(); await turn();
  assert.equal(closing.settled, false);
  h.failure = true; worker.stop.reject(new Error('private fixture failure'));
  await closing.done;
  assert.equal(closing.error.message, 'worker cleanup');
  assert.equal(h.bridge.close(), first);
  assert.equal(worker.calls, 1);
});

test('close joins preparation without starting a worker', { timeout: 3000 }, async t => {
  const h = await fixture(t); h.start();
  const closing = observe(h.bridge.close()); await turn();
  assert.equal(closing.settled, false);
  h.load(); await closing.done;
  assert.equal(closing.error, undefined);
  assert.equal(h.threads.length, 0);
  assert.throws(() => h.start());
});

test('an exited worker does not require another stop', { timeout: 3000 }, async t => {
  const h = await fixture(t); h.start(); h.load(); await turn();
  const worker = h.threads[0]; worker.exit();
  const closing = observe(h.bridge.close()); await closing.done;
  assert.equal(closing.error, undefined);
  assert.equal(worker.calls, 0);
  assert.equal(h.bridge.stats().errors, 1);
});
