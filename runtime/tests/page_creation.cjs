'use strict';
const assert = require('node:assert/strict');
const {test} = require('node:test');
const {runInNewContext} = require('node:vm');
const fs = require('node:fs');
const source = fs.readFileSync(process.argv[2], 'utf8');
const guard = fs.readFileSync(process.argv[3], 'utf8');

class Receiver {
  constructor() { this.listeners = new Map(); }
  addEventListener(name, listener) {
    if (!this.listeners.has(name)) this.listeners.set(name, new Set());
    this.listeners.get(name).add(listener);
  }
  removeEventListener(name, listener) { this.listeners.get(name)?.delete(listener); }
  emit(name, value) { for (const listener of this.listeners.get(name) || []) listener(value); }
  count() { return [...this.listeners.values()].reduce((n, set) => n + set.size, 0); }
}

function fixture() {
  const observers = new Receiver();
  const windows = [];
  const services = {obs: {
    addObserver: (callback, topic) => observers.addEventListener(topic, callback),
    removeObserver: (callback, topic) => observers.removeEventListener(topic, callback)
  }, scriptSecurityManager: {getSystemPrincipal() { return {}; }}};
  const Guard = runInNewContext(guard + '; PageCreationGuard', {
    Services: services, setTimeout, clearTimeout
  });
  class FastGuard extends Guard {
    constructor(window) { super(window, 15, 15); }
  }
  let registry;
  services.ww = {openWindow() {
    const window = new Receiver();
    window.closed = false;
    window.document = {readyState: 'complete'};
    window.gBrowser = {browsers: [{focus() {}}]};
    window.close = () => {
      if (registry.refuseClose) return;
      window.closed = true;
      registry._browserToTarget.delete(window.gBrowser.browsers[0]);
      observers.emit('domwindowclosed', window);
    };
    if (!registry.stallStartup) window.delayedStartupPromise = Promise.resolve();
    windows.push(window);
    if (!registry.stallTarget) registry._browserToTarget.set(window.gBrowser.browsers[0], {
      id: () => windows.indexOf(window) + 1
    });
    return window;
  }};
  const start = source.indexOf('  async newPage({browserContextId}) {');
  const end = source.indexOf('\n  targets() {', start);
  assert.ok(start > 0 && end > start);
  const ready = source.slice(source.indexOf('async function waitForWindowReady('),
    source.indexOf('\nTargetRegistry.Events ='));
  const Registry = runInNewContext('let globalNewPageChain = Promise.resolve();\n' +
    ready + '\nclass Registry extends Receiver {' + source.slice(start, end) + '}; Registry', {
      PageCreationGuard: FastGuard, Receiver, Services: services,
      helper: {awaitEvent: (receiver, name) => new Promise(resolve => {
        const listener = () => { receiver.removeEventListener(name, listener); resolve(); };
        receiver.addEventListener(name, listener);
      })},
      TargetRegistry: {Events: {TargetCreated: 'target'}},
      AppConstants: {BROWSER_CHROME_URL: 'fixture'}, Ci: {},
      Cc: new Proxy({}, {get: () => ({createInstance: () => ({appendElement() {}})})})
    });
  registry = new Registry();
  registry._browserToTarget = new Map();
  registry.browserContextForId = () => ({userContextId: 1, crossProcessCookie: {settings: {}}});
  return {registry, windows, observers};
}

test('startup timeout closes its window, removes observers and releases the queue', {timeout: 1000}, async () => {
  const {registry, windows, observers} = fixture();
  registry.stallStartup = true;
  const failed = registry.newPage({browserContextId: 'fixture'});
  await assert.rejects(failed, /timed out.*browser window startup/);
  assert.equal(windows[0].closed, true);
  assert.equal(observers.count(), 0);
  registry.stallStartup = false;
  assert.equal(await registry.newPage({browserContextId: 'fixture'}), 2);
  observers.emit('browser-delayed-startup-finished', windows[0]);
  assert.equal(registry._browserToTarget.size, 1);
  assert.equal(observers.count(), 0);
});

test('missing page target times out without retaining registry listeners', {timeout: 1000}, async () => {
  const {registry, windows, observers} = fixture();
  registry.stallTarget = true;
  await assert.rejects(registry.newPage({browserContextId: 'fixture'}), /page target registration/);
  assert.equal(windows[0].closed, true);
  assert.equal(registry.count(), 0);
  assert.equal(observers.count(), 0);
  registry.stallTarget = false;
  assert.equal(await registry.newPage({browserContextId: 'fixture'}), 2);
});

test('a window closed during startup releases its queued successor', async () => {
  const {registry, windows, observers} = fixture();
  registry.stallStartup = true;
  const failed = registry.newPage({browserContextId: 'fixture'});
  await new Promise(resolve => setImmediate(resolve));
  registry.stallStartup = false;
  const next = registry.newPage({browserContextId: 'fixture'});
  windows[0].close();
  await assert.rejects(failed, /window closed during/);
  assert.equal(await next, 2);
  assert.equal(observers.count(), 0);
});

test('unconfirmed close refuses later allocation with the actual cleanup cause', async () => {
  const {registry, windows, observers} = fixture();
  registry.stallStartup = registry.refuseClose = true;
  await assert.rejects(registry.newPage({browserContextId: 'fixture'}), /cleanup unconfirmed.*did not confirm closure/);
  await assert.rejects(registry.newPage({browserContextId: 'fixture'}), /cleanup unconfirmed/);
  assert.equal(windows.length, 1);
  assert.equal(observers.count(), 0);
});

test('a failed new page does not close an existing active page', async () => {
  const {registry, windows, observers} = fixture();
  assert.equal(await registry.newPage({browserContextId: 'fixture'}), 1);
  registry.stallStartup = true;
  await assert.rejects(registry.newPage({browserContextId: 'fixture'}), /timed out/);
  assert.equal(windows[0].closed, false);
  assert.equal(windows[1].closed, true);
  assert.equal(registry._browserToTarget.size, 1);
  assert.equal(observers.count(), 0);
});
