// Inert ports only; no SDK import or real worker.
// node --experimental-vm-modules --test scripts/offline_worker/dedicated_pool_tests.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { Buffer } from 'node:buffer';
import { createHash } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const source = Buffer.from('// inert worker fixture');
const digest = createHash('sha256').update(source).digest('hex');
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
  const threads = [], reads = [];
  class Thread extends EventEmitter {
    constructor() {
      super(); threads.push(this); this.termination = deferred(); this.calls = 0;
      this.stdout = this.stderr = { resume() {} };
    }
    terminate() {
      this.calls++;
      if (this.throwOnTerminate) throw new Error('private fixture failure');
      return this.termination.promise;
    }
    exit() { if (!this.exited) { this.exited = true; this.emit('exit', 0); } }
  }
  const url = new URL('./dedicated_pool.mjs', import.meta.url);
  const context = vm.createContext({ Buffer, URL });
  const module = new vm.SourceTextModule(await readFile(url, 'utf8'), {
    context, initializeImportMeta(meta) { meta.url = url.href; },
  });
  const imports = {
    'node:crypto': { createHash }, 'node:worker_threads': { Worker: Thread },
    './dedicated_replies.mjs': { classifyReply() { throw new Error('unexpected reply'); } },
  };
  await module.link(name => {
    assert.ok(Object.hasOwn(imports, name), 'unexpected import');
    const entries = Object.entries(imports[name]);
    return new vm.SyntheticModule(entries.map(([key]) => key), function () {
      for (const [key, value] of entries) this.setExport(key, value);
    }, { context });
  });
  await module.evaluate();
  class DataEvent {
    constructor(type, options) { this.type = type; Object.assign(this, options); }
  }
  const pool = module.namespace.createPool({ navigator: { userAgent: 'Framerize/0.1' },
    ErrorEvent: DataEvent, MessageEvent: DataEvent }, new Set([digest]));
  const h = { pool, threads, cleanupFailure: false,
    start() {
      const read = deferred(), events = []; reads.push(read);
      const slot = pool.start({ arrayBuffer: () => read.promise },
        { dispatchEvent: event => events.push(event) });
      return { slot, events, load: (bytes = source) => read.resolve(bytes) };
    },
    fail(thread) {
      h.cleanupFailure = true;
      thread.termination.reject(new Error('private fixture failure'));
    },
  };
  t.after(async () => {
    const closing = observe(pool.close());
    for (const read of reads) read.resolve(source);
    await turn();
    for (const thread of threads) { thread.termination.resolve(0); thread.exit(); }
    await closing.done;
    assert.equal(Boolean(closing.error), h.cleanupFailure);
  });
  return h;
}

test('failed stop waits for both worker exits before close rejects', { timeout: 3000 }, async t => {
  const h = await fixture(t), a = h.start(), b = h.start();
  a.load(); b.load(); await turn();
  const first = h.pool.close(), closing = observe(first);
  assert.equal(first, h.pool.close()); await turn();
  h.fail(h.threads[0]); await turn();
  assert.equal(closing.settled, false, 'rejection alone is not joined cleanup');
  h.threads[0].exit(); await turn();
  assert.equal(closing.settled, false, 'sibling stop is still pending');
  h.threads[1].termination.resolve(0); await turn();
  assert.equal(closing.settled, false, 'sibling exit is still pending');
  h.threads[1].exit(); await closing.done;
  assert.equal(closing.error?.message, 'worker cleanup');
  assert.deepEqual(h.threads.map(thread => thread.calls), [1, 1]);
  assert.equal(h.pool.stats().joined, 2);
});

test('successful termination still waits for exit', { timeout: 3000 }, async t => {
  const h = await fixture(t), a = h.start(), b = h.start();
  a.load(); b.load(); await turn();
  const closing = observe(h.pool.close()); await turn();
  for (const thread of h.threads) thread.termination.resolve(0);
  await turn(); assert.equal(closing.settled, false);
  h.threads[0].exit(); await turn(); assert.equal(closing.settled, false);
  h.threads[1].exit(); await closing.done;
  assert.equal(closing.error, undefined);
  assert.deepEqual({ ...closing.value }, { started: 2, joined: 2, errors: 0, messages: 0 });
  assert.deepEqual(h.threads.map(thread => thread.calls), [1, 1]);
});

test('exit does not erase a pending termination result', { timeout: 3000 }, async t => {
  const h = await fixture(t), a = h.start(); a.load(); await turn();
  const closing = observe(h.pool.close()); await turn();
  h.threads[0].exit(); await turn();
  assert.equal(h.pool.stats().joined, 1); assert.equal(closing.settled, false);
  h.fail(h.threads[0]); await closing.done;
  assert.equal(closing.error?.message, 'worker cleanup');
});

test('discarded stop rejection is retained by later close', { timeout: 3000 }, async t => {
  const h = await fixture(t), a = h.start(); a.load(); await turn();
  // Match the browser adapter, which cannot return an async termination result.
  const stop = a.slot.stop();
  assert.equal(a.slot.stop(), stop); await turn();
  h.fail(h.threads[0]); h.threads[0].exit(); await turn();
  const closing = observe(h.pool.close()); await closing.done;
  assert.equal(closing.error?.message, 'worker cleanup');
  assert.equal(h.threads[0].calls, 1);
});

test('synchronous termination failure still waits for exit', { timeout: 3000 }, async t => {
  const h = await fixture(t), a = h.start(); a.load(); await turn();
  h.cleanupFailure = true; h.threads[0].throwOnTerminate = true;
  const closing = observe(h.pool.close()); await turn();
  assert.equal(closing.settled, false);
  h.threads[0].exit(); await closing.done;
  assert.equal(closing.error?.message, 'worker cleanup');
  assert.equal(h.threads[0].calls, 1);
});

test('close during source preparation prevents launch', { timeout: 3000 }, async t => {
  const h = await fixture(t), a = h.start(), b = h.start();
  const first = h.pool.close(), closing = observe(first);
  assert.equal(first, h.pool.close());
  a.load(); await turn(); assert.equal(closing.settled, false);
  b.load(); await closing.done;
  assert.equal(closing.error, undefined); assert.equal(h.threads.length, 0);
  assert.deepEqual({ ...h.pool.stats() }, { started: 0, joined: 0, errors: 0, messages: 0 });
  assert.throws(() => h.start(), { message: 'worker limit' });
});

test('changed source refuses before launch', { timeout: 3000 }, async t => {
  const h = await fixture(t), a = h.start();
  a.load(Buffer.from('changed fixture source')); await turn();
  assert.deepEqual(a.events.map(event => event.message), ['worker failed']);
  const closing = observe(h.pool.close()); await closing.done;
  assert.equal(closing.error, undefined); assert.equal(h.threads.length, 0);
  assert.deepEqual({ ...h.pool.stats() }, { started: 0, joined: 0, errors: 1, messages: 0 });
});

test('an already exited worker needs no new termination', { timeout: 3000 }, async t => {
  const h = await fixture(t), a = h.start(); a.load(); await turn();
  h.threads[0].exit();
  const closing = observe(h.pool.close()); await closing.done;
  assert.equal(closing.error, undefined); assert.equal(h.threads[0].calls, 0);
  assert.deepEqual(a.events.map(event => event.message), ['worker failed']);
  assert.deepEqual({ ...h.pool.stats() }, { started: 1, joined: 1, errors: 1, messages: 0 });
});
