import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { DOM } from '../../scripts/offline_worker/pins.mjs';
import { createV2Window } from '../../scripts/offline_worker/v2_runtime.mjs';
import { installV2DedicatedWorkers } from '../../scripts/offline_worker/v2_dedicated_workers.mjs';
const jsdom = createRequire(import.meta.url)(DOM + '/node_modules/jsdom/lib/api.js');
const source = 'self.onmessage=e=>postMessage({value:e.data,ua:navigator.userAgent,clock:typeof performance.now()});';
const digest = createHash('sha256').update(source).digest('hex');
const windowForTest = () => createV2Window(jsdom, 'https://framer.com/projects/fixture', new jsdom.VirtualConsole());
function messageFrom(worker) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('native worker reply timeout')), 2000);
    worker.onmessage = event => { clearTimeout(timer); resolve(event.data); };
    worker.onerror = () => { clearTimeout(timer); reject(new Error('native worker failed')); };
  });
}
function snapshot(window) {
  return {Worker:window.Worker, Blob:window.Blob, create:window.URL.createObjectURL,
    revoke:window.URL.revokeObjectURL, navigator:window.navigator, ua:window.navigator.userAgent};
}
function restored(window, before) {
  assert.deepEqual(snapshot(window), before);
}
function workerFor(window) {
  const blob = new window.Blob([source], {type:'application/javascript'});
  return new window.Worker(window.URL.createObjectURL(blob));
}

test('v2_real_worker_reports_native_navigator_and_joins_before_restoring_hooks', async () => {
  const dom = windowForTest(), before = snapshot(dom.window);
  let owner;
  try {
    owner = installV2DedicatedWorkers(dom.window, digest);
    const worker = workerFor(dom.window), reply = messageFrom(worker);
    worker.postMessage('');
    assert.deepEqual(await reply, {value:'',ua:globalThis.navigator.userAgent,clock:'number'});
    assert.equal(dom.window.navigator, before.navigator);
    assert.equal(dom.window.navigator.userAgent, before.ua);
    await owner.close();
    const stats = owner.stats();
    assert.equal(stats.started, 1);
    assert.equal(stats.joined, 1);
    assert.equal(stats.errors, 0);
    restored(dom.window, before);
  } finally { if (owner) await owner.close(); dom.window.close(); }
});

test('v2_immediate_worker_close_drains_accepted_start_and_restores_hooks', async () => {
  const dom = windowForTest(), before = snapshot(dom.window);
  let owner;
  try {
    owner = installV2DedicatedWorkers(dom.window, digest);
    workerFor(dom.window);
    await Promise.all([owner.close(), owner.close()]);
    const stats = owner.stats();
    assert.equal(stats.started, stats.joined);
    assert.ok(stats.started === 0 || stats.started === 1);
    restored(dom.window, before);
  } finally { if (owner) await owner.close(); dom.window.close(); }
});
