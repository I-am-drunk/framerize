'use strict';
const assert = require('node:assert/strict');
const {test} = require('node:test');
const {runInNewContext} = require('node:vm');
const source = require('node:fs').readFileSync(process.argv[2], 'utf8');

test('native cleanup coalesces overlapping closes and respects its interval', async () => {
  let now = 0, enabled = true, calls = 0, finish;
  const start = source.indexOf('let framerizeLastReclaim');
  const end = source.indexOf('const IDENTITY_NAME', start);
  assert.ok(start >= 0 && end > start);
  const reclaim = runInNewContext(source.slice(start, end) + '; reclaimClosedContexts', {
    Date: {now: () => now},
    Services: {prefs: {getBoolPref: () => enabled}},
    Ci: {nsIMemoryReporterManager: 'fixture'},
    Cc: {'@mozilla.org/memory-reporter-manager;1': {getService: () => ({
      minimizeMemoryUsage: callback => {calls++; finish = callback;}
    })}}
  });
  const first = reclaim();
  assert.equal(reclaim(), first);
  assert.equal(calls, 1);
  finish();
  await first;
  now = 4999;
  assert.equal(reclaim(), undefined);
  now = 5000;
  const next = reclaim();
  assert.equal(calls, 2);
  finish();
  await next;
  enabled = false;
  now = 20000;
  assert.equal(reclaim(), undefined);
  assert.equal(calls, 2);
});

test('closing one page drops only its actor, channel and shared entries', () => {
  const start = source.indexOf('  dispose() {', source.indexOf('export class PageTarget'));
  const end = source.indexOf('\n}\n\nPageTarget.Events', start);
  assert.ok(start > 0 && end > start);
  const shared = new Map([['juggler:page-cookie-1', {}], ['juggler:page-cookie-2', {}]]);
  const Target = runInNewContext('class Target {' + source.slice(start, end) + '}; Target', {
    Services: {ppmm: {sharedData: {delete: key => shared.delete(key), flush() {}}}},
    helper: {removeListeners() {}}, TargetRegistry: {Events: {TargetDestroyed: 'closed'}}
  });
  const target = new Target();
  let closes = 0;
  const other = {};
  Object.assign(target, {
    _linkedBrowser: {browsingContext: {browserId: 1}},
    _channel: {dispose() {closes++;}}, _eventListeners: [],
    ensureContextMenuClosed() {},
    _browserContext: {pages: new Set([target])},
    _registry: {_browserIdToActor: new Map([[1, {}], [2, other]]),
      _browserIdToTarget: new Map([[1, target], [2, other]]),
      _browserToTarget: new Map(), emit() {}}
  });
  target.dispose();
  target.dispose();
  assert.equal(closes, 1);
  assert.equal(target._browserContext.pages.size, 0);
  assert.equal(target._registry._browserIdToActor.size, 1);
  assert.equal(target._registry._browserIdToTarget.get(2), other);
  assert.deepEqual([...shared.keys()], ['juggler:page-cookie-2']);
});
