// Inert ports only. Run with node --experimental-vm-modules --test <this file>.
import test from 'node:test';
import assert from 'node:assert/strict';
import { Blob, Buffer, resolveObjectURL } from 'node:buffer';
import { createHash } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { readFile } from 'node:fs/promises';
import { performance } from 'node:perf_hooks';
import { URL } from 'node:url';
import vm from 'node:vm';

const hash = source => createHash('sha256').update(source).digest('hex');
const script = 'self.onmessage=e=>postMessage([e.data,navigator.userAgent]);';
const turn = () => new Promise(resolve => setImmediate(resolve));
async function load(name, ports, globals = {}) {
  const url = new URL('../../scripts/offline_worker/' + name, import.meta.url);
  const context = vm.createContext(globals);
  const module = new vm.SourceTextModule(await readFile(url, 'utf8'), {
    context, initializeImportMeta(meta) { meta.url = url.href; },
  });
  const imports = { 'node:buffer': { Blob, Buffer }, 'node:crypto': { createHash },
    'node:url': { URL }, 'node:perf_hooks': { performance },
    'node:vm': { default: vm }, 'node:worker_threads': ports };
  await module.link(name => {
    assert.ok(Object.hasOwn(imports, name), 'unexpected import');
    const entries = Object.entries(imports[name]);
    return new vm.SyntheticModule(entries.map(([key]) => key), function () {
      for (const [key, value] of entries) this.setExport(key, value);
    }, { context });
  });
  await module.evaluate();
  return module.namespace;
}
function fakeWindow() {
  class DataEvent extends Event {
    constructor(type, options) { super(type); Object.assign(this, options); }
  }
  const navigator = Object.freeze({ userAgent: 'unchanged-window', platform: 'fixture' });
  const window = { location: { origin: 'https://framer.com' },
    EventTarget, MessageEvent: DataEvent, ErrorEvent: DataEvent, DOMException,
    Worker: class OriginalWorker {}, Blob: class OriginalBlob {}, URL: class OriginalURL {} };
  Object.defineProperty(window, 'navigator', { get: () => navigator, configurable: true });
  Object.defineProperty(window.URL, 'createObjectURL', {
    value: () => 'original', configurable: true, writable: false,
  });
  return window;
}
const descriptors = window => [Object.getOwnPropertyDescriptors(window),
  Object.getOwnPropertyDescriptors(window.URL)];
async function fixture(t, source = script, admitted = source, { closeFailure = false } = {}) {
  const threads = [], window = fakeWindow(), before = descriptors(window);
  const launches = new EventEmitter();
  let resolveStarted;
  const started = new Promise(resolve => { resolveStarted = resolve; });
  class Thread extends EventEmitter {
    constructor(url, options) {
      super(); this.url = url; this.options = options; this.posts = []; this.stops = 0;
      this.stdout = this.stderr = { resume() {} };
      threads.push(this); resolveStarted(this);
      launches.emit('started', this);
    }
    postMessage(value) { this.posts.push(value); }
    terminate() { this.stops++; return Promise.resolve(0); }
    exit() { if (!this.exited) { this.exited = true; this.emit('exit', 0); } }
  }
  const { installV2DedicatedWorkers: install } = await load('v2_dedicated_workers.mjs', { Worker: Thread });
  const bridge = install(window, hash(admitted));
  const url = window.URL.createObjectURL(new window.Blob([source], { type: 'application/javascript' }));
  t.after(async () => {
    const closing = bridge.close();
    for (const thread of threads) thread.exit();
    if (closeFailure) await assert.rejects(closing, /^Error: worker cleanup$/);
    else await closing;
  });
  return { window, before, bridge, url, started, threads, install,
    nextStarted: () => new Promise(resolve => launches.once('started', resolve)) };
}
const check = (name, run) => test(name, { timeout: 5000 }, run);

function terminationGate(thread) {
  let enter, resolve, reject;
  const entered = new Promise(done => { enter = done; });
  const pending = new Promise((done, fail) => { resolve = done; reject = fail; });
  thread.terminate = () => { thread.stops++; enter(); return pending; };
  return { entered, resolve, reject };
}

check('v2_worker_scope_and_digest_refuse_without_hook_mutation', async t => {
  const h = await fixture(t), window = fakeWindow(), before = descriptors(window);
  for (const digest of [null, {}, '', 'A'.repeat(64), 'a'.repeat(63)])
    assert.throws(() => h.install(window, digest), /worker scope/);
  window.location.origin = 'https://example.invalid';
  assert.throws(() => h.install(window, hash(script)), /worker scope/);
  assert.throws(() => h.install(h.window, hash(script)), /worker scope/);
  assert.deepEqual(descriptors(window), before);
});

check('v2_worker_uses_native_blobs_and_bounded_registered_urls', async t => {
  const h = await fixture(t), { window } = h;
  assert.equal(window.Blob, Blob);
  assert.equal(await resolveObjectURL(h.url).text(), script);
  for (const blob of [{ size: 1 }, new Blob([]), new Blob(['é'.repeat(2049)])])
    assert.throws(() => window.URL.createObjectURL(blob), /unsupported worker blob/);
  for (let i = 0; i < 3; i++) window.URL.createObjectURL(new Blob(['x'.repeat(4096)]));
  assert.throws(() => window.URL.createObjectURL(new Blob(['x'])), /unsupported worker blob/);
  for (const url of ['https://framer.com/worker.js', 'file:///tmp/worker.js',
    'data:text/javascript,', 'blob:nodedata:unregistered', null])
    assert.throws(() => new window.Worker(url));
  window.URL.revokeObjectURL(h.url);
  assert.equal(resolveObjectURL(h.url), undefined);
  assert.throws(() => new window.Worker(h.url));
});

check('v2_worker_rejects_source_digest_before_thread_creation', async t => {
  const h = await fixture(t, script + '// private-marker', script);
  const worker = new h.window.Worker(h.url);
  const error = await new Promise(resolve => { worker.onerror = resolve; });
  assert.equal(error.message, 'worker failed');
  assert.equal(error.error, undefined);
  assert.equal(h.threads.length, 0);
  assert.deepEqual({ ...h.bridge.stats() }, { started: 0, joined: 0, errors: 1, messages: 0 });
});

check('v2_worker_forwards_one_message_with_bounded_identity_free_launch', async t => {
  const h = await fixture(t), worker = new h.window.Worker(h.url), received = [];
  worker.onmessage = event => received.push(event.data);
  assert.throws(() => worker.postMessage('not-empty'));
  assert.throws(() => worker.postMessage('', []));
  worker.postMessage('');
  assert.throws(() => worker.postMessage(''));
  const thread = await h.started; await turn();
  assert.deepEqual(thread.posts, ['']);
  assert.match(thread.url.pathname, /\/v2_dedicated_runtime\.mjs$/);
  assert.deepEqual({ ...thread.options.workerData }, { source: script, sha256: hash(script) });
  assert.deepEqual({ ...thread.options.resourceLimits }, {
    maxOldGenerationSizeMb: 16, maxYoungGenerationSizeMb: 4, stackSizeMb: 2,
  });
  assert.deepEqual(Array.from(thread.options.execArgv), []);
  assert.equal(thread.options.stdout, true); assert.equal(thread.options.stderr, true);
  thread.emit('message', [1, true]);
  assert.deepEqual(received, [[1, true]]);
  assert.equal(h.bridge.stats().messages, 1);
});

check('v2_worker_options_and_second_live_worker_are_refused', async t => {
  const h = await fixture(t);
  for (const options of [null, {}, { type: 'module' }, { name: 'fixture' }])
    assert.throws(() => new h.window.Worker(h.url, options));
  const worker = new h.window.Worker(h.url);
  assert.throws(() => new h.window.Worker(h.url), /Worker limit/);
  const thread = await h.started;
  worker.terminate(); await turn();
  assert.throws(() => new h.window.Worker(h.url), /Worker limit/);
  assert.equal(thread.stops, 1);
  thread.exit();
  assert.equal(h.bridge.stats().joined, 1);
  new h.window.Worker(h.url);
});

check('v2_worker_close_before_preparation_prevents_spawn_and_restores_hooks', async t => {
  const h = await fixture(t), worker = new h.window.Worker(h.url);
  worker.postMessage('');
  const closing = h.bridge.close();
  worker.terminate(); worker.terminate();
  assert.equal(closing, h.bridge.close());
  await closing;
  assert.deepEqual({ ...h.bridge.stats() }, { started: 0, joined: 0, errors: 0, messages: 0 });
  assert.deepEqual(descriptors(h.window), h.before);
  assert.equal(resolveObjectURL(h.url), undefined);
});

check('v2_worker_close_waits_for_exit_after_termination_promise', async t => {
  const h = await fixture(t), worker = new h.window.Worker(h.url);
  const thread = await h.started;
  let settled = false;
  const closing = h.bridge.close(); closing.then(() => { settled = true; });
  worker.terminate(); worker.terminate();
  assert.equal(closing, h.bridge.close());
  await turn();
  assert.equal(thread.stops, 1); assert.equal(settled, false);
  assert.equal(h.bridge.stats().joined, 0);
  thread.exit(); await closing;
  assert.deepEqual({ ...h.bridge.stats() }, { started: 1, joined: 1, errors: 0, messages: 0 });
  assert.deepEqual(descriptors(h.window), h.before);
});

check('v2_worker_close_waits_for_termination_after_exit_removed_live_slot', async t => {
  const h = await fixture(t), worker = new h.window.Worker(h.url);
  const thread = await h.started, gate = terminationGate(thread);
  worker.terminate(); await gate.entered; thread.exit();
  let settled = false;
  const closing = h.bridge.close();
  closing.then(() => { settled = true; }, () => { settled = true; });
  try {
    await turn();
    assert.equal(settled, false);
    assert.equal(h.bridge.stats().joined, 1);
    assert.notEqual(h.window.Worker, h.before[0].Worker.value);
    assert.equal(closing, h.bridge.close());
    worker.terminate(); assert.equal(thread.stops, 1);
  } finally { gate.resolve(0); await closing; }
  assert.deepEqual(descriptors(h.window), h.before);
  assert.deepEqual({ ...h.bridge.stats() }, { started: 1, joined: 1, errors: 0, messages: 0 });
});

check('v2_worker_replacement_preserves_old_cleanup_and_one_live_admission', async t => {
  const h = await fixture(t), worker = new h.window.Worker(h.url);
  const first = await h.started, gate = terminationGate(first);
  worker.terminate(); await gate.entered;
  try {
    assert.throws(() => new h.window.Worker(h.url), /Worker limit/);
    first.exit();
    const started = h.nextStarted();
    new h.window.Worker(h.url);
    const second = await started;
    assert.throws(() => new h.window.Worker(h.url), /Worker limit/);
    let settled = false;
    const closing = h.bridge.close();
    closing.then(() => { settled = true; }, () => { settled = true; });
    await turn();
    assert.equal(second.stops, 1);
    second.exit(); await turn();
    assert.equal(settled, false);
    assert.equal(h.bridge.stats().joined, 2);
    assert.notEqual(h.window.Worker, h.before[0].Worker.value);
  } finally {
    gate.resolve(0);
    const closing = h.bridge.close();
    for (const thread of h.threads) thread.exit();
    await closing;
  }
  assert.deepEqual(descriptors(h.window), h.before);
  assert.deepEqual({ ...h.bridge.stats() }, { started: 2, joined: 2, errors: 0, messages: 0 });
});

check('v2_worker_failed_termination_still_waits_for_exit', async t => {
  const h = await fixture(t, script, script, { closeFailure: true });
  new h.window.Worker(h.url);
  const thread = await h.started, gate = terminationGate(thread);
  let settled = false;
  const closing = h.bridge.close();
  closing.then(() => { settled = true; }, () => { settled = true; });
  await gate.entered; gate.reject(new Error('private-marker'));
  try {
    await turn();
    assert.equal(settled, false);
    assert.equal(h.bridge.stats().joined, 0);
    assert.notEqual(h.window.Worker, h.before[0].Worker.value);
  } finally { thread.exit(); await assert.rejects(closing, /^Error: worker cleanup$/); }
  assert.deepEqual(descriptors(h.window), h.before);
  assert.equal(h.bridge.stats().joined, 1);
  assert.equal(h.bridge.stats().errors, 0);
});

for (const timing of ['before_close', 'during_close'])
check('v2_worker_retains_termination_failure_after_exit_' + timing, async t => {
  const h = await fixture(t, script, script, { closeFailure: true });
  const worker = new h.window.Worker(h.url);
  const thread = await h.started, gate = terminationGate(thread);
  worker.terminate(); await gate.entered; thread.exit();
  if (timing === 'before_close') { gate.reject(new Error('private-marker')); await turn(); }
  const closing = h.bridge.close();
  const rejected = assert.rejects(closing, /^Error: worker cleanup$/);
  if (timing === 'during_close') gate.reject(new Error('private-marker'));
  await rejected;
  assert.equal(closing, h.bridge.close());
  assert.equal(thread.stops, 1);
  assert.deepEqual(descriptors(h.window), h.before);
  assert.deepEqual({ ...h.bridge.stats() }, { started: 1, joined: 1, errors: 0, messages: 0 });
});

check('v2_worker_cleanup_failure_isolated_between_two_windows', async t => {
  const h = await fixture(t, script, script, { closeFailure: true });
  const window = fakeWindow(), before = descriptors(window);
  const owner = h.install(window, hash(script));
  const url = window.URL.createObjectURL(new window.Blob([script]));
  t.after(async () => {
    const closing = owner.close();
    for (const thread of h.threads) thread.exit();
    await closing;
  });
  const firstWorker = new h.window.Worker(h.url), first = await h.started;
  assert.throws(() => new h.window.Worker(url));
  assert.throws(() => new window.Worker(h.url));
  h.window.URL.revokeObjectURL(url);
  assert.ok(resolveObjectURL(url));
  const started = h.nextStarted(), secondWorker = new window.Worker(url);
  const second = await started, gate = terminationGate(first);
  firstWorker.terminate(); await gate.entered; first.exit();
  const rejected = assert.rejects(h.bridge.close(), /^Error: worker cleanup$/);
  try {
    assert.ok(resolveObjectURL(url));
    await turn();
    assert.equal(second.stops, 0);
    assert.notEqual(window.Worker, before[0].Worker.value);
    const messages = [];
    secondWorker.onmessage = event => messages.push(event.data);
    second.emit('message', 'second-window');
    assert.deepEqual(messages, ['second-window']);
    assert.equal(h.bridge.stats().messages, 0);
    gate.reject(new Error('private-marker')); await rejected;
    assert.equal(second.stops, 0);
    const closing = owner.close();
    await turn(); assert.equal(second.stops, 1);
    second.exit(); await closing;
    assert.deepEqual(descriptors(window), before);
    assert.deepEqual(descriptors(h.window), h.before);
    assert.deepEqual({ ...owner.stats() }, { started: 1, joined: 1, errors: 0, messages: 1 });
    assert.deepEqual({ ...h.bridge.stats() }, { started: 1, joined: 1, errors: 0, messages: 0 });
  } finally {
    gate.reject(new Error('private-marker'));
    const closing = owner.close();
    for (const thread of h.threads) thread.exit();
    await Promise.all([rejected, closing]);
  }
});

check('v2_worker_errors_are_generic_and_still_joined', async t => {
  const h = await fixture(t), worker = new h.window.Worker(h.url), errors = [];
  worker.onerror = event => errors.push([event.message, event.error]);
  const thread = await h.started;
  thread.emit('error', new Error('private-marker'));
  thread.emit('error', new Error('another-private-marker'));
  thread.emit('message', 'ignored-after-error');
  const closing = h.bridge.close(); thread.exit(); await closing;
  assert.deepEqual(errors, [['worker failed', undefined]]);
  assert.deepEqual({ ...h.bridge.stats() }, { started: 1, joined: 1, errors: 1, messages: 0 });
});

check('v2_worker_restoration_preserves_replacement_hooks_and_foreign_urls', async t => {
  const h = await fixture(t), replacement = class ReplacementWorker {};
  const foreign = URL.createObjectURL(new Blob(['foreign']));
  try {
    assert.throws(() => new h.window.Worker(foreign));
    h.window.URL.revokeObjectURL(foreign);
    assert.ok(resolveObjectURL(foreign));
    h.window.Worker = replacement;
    await h.bridge.close();
    assert.equal(h.window.Worker, replacement);
    assert.equal(Object.hasOwn(h.window.URL, 'revokeObjectURL'), false);
    assert.ok(resolveObjectURL(foreign));
  } finally { URL.revokeObjectURL(foreign); }
});

check('v2_worker_nonconfigurable_hook_refuses_atomically', async t => {
  const h = await fixture(t), window = fakeWindow();
  Object.defineProperty(window.URL, 'revokeObjectURL', { value() {}, configurable: false });
  const before = descriptors(window);
  assert.throws(() => h.install(window, hash(script)), /worker hooks/);
  assert.deepEqual(descriptors(window), before);
});

async function child(source, data = {}, globals = { navigator: globalThis.navigator }) {
  const port = new EventEmitter(), replies = [];
  port.postMessage = value => replies.push(value);
  await load('v2_dedicated_runtime.mjs', {
    parentPort: port, workerData: { source, sha256: hash(source), ...data },
  }, globals);
  return { port, replies };
}

check('v2_worker_child_rechecks_hash_size_and_exact_input_fields', async () => {
  await assert.rejects(child(script, { sha256: '0'.repeat(64) }), /worker source/);
  await assert.rejects(child(script + ' '.repeat(4096)), /worker source/);
  await assert.rejects(child(script, { userAgent: 'spoof' }), /worker input/);
  await assert.rejects(child(script, {}, {}), /worker navigator/);
  await assert.rejects(child('throw Error("private-marker")'), /^Error: worker execution$/);
});

check('v2_worker_child_uses_genuine_node_objects_without_browser_fabrication', async () => {
  const h = await child('self.onmessage=()=>postMessage([navigator,performance,' +
    'typeof fetch,typeof process,typeof require,typeof OffscreenCanvas,typeof history]);');
  h.port.emit('message', '');
  assert.equal(h.replies[0][0], globalThis.navigator);
  assert.equal(h.replies[0][1], performance);
  assert.deepEqual(Array.from(h.replies[0]).slice(2), Array(5).fill('undefined'));
  assert.throws(() => h.port.emit('message', ''), /worker message/);
});

check('v2_worker_child_rejects_nonempty_messages_and_dynamic_code', async () => {
  const h = await child(script);
  assert.throws(() => h.port.emit('message', 'private-marker'), /^Error: worker message$/);
  assert.equal(h.replies.length, 0);
  const dynamic = await child('self.onmessage=()=>eval("1")');
  assert.throws(() => dynamic.port.emit('message', ''), /^Error: worker message failed$/);
});
