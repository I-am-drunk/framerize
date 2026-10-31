// Observation-only trace: run the production module chain under JSDOM with
// prototype-level instrumentation and record every SDK API access.
// No network: JSDOM has no resource loader here, and mint is never called.
import fs from 'node:fs';
import { createRequire } from 'node:module';
import { TextEncoder } from 'node:util';
import { DOM, PUBLIC_KEY, ASSETS, MODULES, CDN } from '../pins.mjs';
import { collectionSource } from '../collection.mjs';
import { makeTraceState, makeWrapper, installOnWindow, record } from './trace_support.mjs';
import vm from 'node:vm';
import { createHash } from 'node:crypto';

const OUT = new URL('./trace-output.json', import.meta.url).pathname;
const state = makeTraceState();
const wrap = makeWrapper(state);
const require = createRequire(import.meta.url);
const jsdom = require(DOM + '/node_modules/jsdom/lib/api.js');
const virtualConsole = new jsdom.VirtualConsole();
const consoleErrors = [];
virtualConsole.on('error', e => consoleErrors.push(String(e)));
virtualConsole.on('jsdomError', e => consoleErrors.push(String(e && e.message)));

const dom = new jsdom.JSDOM('<!doctype html><html><head></head><body></body></html>', {
  url: 'https://app.framer.com/',
  runScripts: 'outside-only',
  pretendToBeVisual: true,
  virtualConsole,
});
const win = dom.window;
// Count canvas usage: instrument createElement to wrap returned canvases.
const realCreate = win.document.createElement.bind(win.document);
let canvasCount = 0;
win.document.createElement = function(tag, ...rest) {
  const el = realCreate(tag, ...rest);
  if (String(tag).toLowerCase() === 'canvas') {
    canvasCount++;
    for (const m of ['getContext', 'toDataURL', 'toBlob']) {
      const orig = el[m] && el[m].bind(el);
      if (!orig) continue;
      el[m] = function(...args) {
        record(state, 'document.createElement(canvas).' + m, 'call', String(args[0] && typeof args[0] === 'string' ? args[0] : ''));
        return orig(...args);
      };
    }
  }
  return el;
};
Object.defineProperty(win, 'TextEncoder', { value: TextEncoder, configurable: true, writable: true });
win.castlePublishableAPIKey = PUBLIC_KEY;
const setState = installOnWindow(win, wrap);
setState(state);

// Instrument event registration so listener adds are visible.
for (const target of [win, win.document]) {
  const proto = Object.getPrototypeOf(target);
  for (const key of ['addEventListener', 'removeEventListener']) {
    const desc = Object.getOwnPropertyDescriptor(win.EventTarget.prototype, key);
    if (!desc) continue;
    if (!win.EventTarget.prototype.__traced) {
      Object.defineProperty(win.EventTarget.prototype, key, {
        configurable: true, writable: true,
        value: function(type, ...rest) {
          record(state, (this === win ? 'window' : 'event-target') + '.' + key + '(' + String(type) + ')', 'call', 'listener');
          return desc.value.call(this, type, ...rest);
        },
      });
    }
  }
  win.EventTarget.prototype.__traced = true;
  break;
}

const context = dom.getInternalVMContext();
context.castlePublishableAPIKey = PUBLIC_KEY;
const modules = new Map();
for (const [role, [name, , hash]] of Object.entries(MODULES)) {
  const bytes = fs.readFileSync(ASSETS + '/' + name);
  if (createHash('sha256').update(bytes).digest('hex') !== hash) throw new Error('input-identity:' + role);
  const source = role === 'v3' ? collectionSource(bytes) : bytes.toString('utf8');
  const module = new vm.SourceTextModule(source, {
    context,
    identifier: CDN + name,
    initializeImportMeta: meta => {
      if (role === 'v3') Object.assign(meta, { collect: v => v, sharedWorker: v => v, network: v => v });
    },
    importModuleDynamically: () => { throw new Error('unexpected-import'); },
  });
  modules.set(role, module);
}
function bySpecifier(specifier) {
  for (const [role, [name]] of Object.entries(MODULES))
    if (specifier === CDN + name) return role;
  throw new Error('unexpected-import:' + specifier);
}
const wrapper = modules.get('wrapper');
await wrapper.link((specifier) => modules.get(bySpecifier(specifier)));
await wrapper.evaluate();
// Trigger mint: the wrapper's export a is the mint function used by the
// production worker. createRequestToken drives the full collection.
const mint = wrapper.namespace.a;
try {
  const token = await Promise.race([
    mint({ budget_ms: 8000 }),
    new Promise((_, reject) => setTimeout(() => reject(new Error('trace-timeout')), 9000)),
  ]);
  console.log('mint-ok type=' + typeof token + ' len=' + (token && token.length));
} catch (e) {
  console.log('mint-failed: ' + (e && e.message));
}
await new Promise(r => setTimeout(r, 500));

const inventory = new Map();
for (const line of state.records) {
  const [path, kind, shape] = line.split('|');
  let entry = inventory.get(path);
  if (!entry) { entry = { gets: 0, calls: 0, shapes: new Set() }; inventory.set(path, entry); }
  if (kind === 'get') entry.gets++;
  else if (kind === 'call') entry.calls++;
  else entry.shapes.add(shape);
}
fs.writeFileSync(OUT, JSON.stringify({
  raw_records: state.records.length,
  truncated: state.truncated,
  console_errors: consoleErrors.slice(0, 50),
  unique_paths: inventory.size,
  inventory: Object.fromEntries([...inventory.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([k, v]) => [k, { gets: v.gets, calls: v.calls, shapes: [...v.shapes] }])),
}, null, 1));
console.log('records=' + state.records.length, 'unique=' + inventory.size,
  'truncated=' + state.truncated, 'errors=' + consoleErrors.length);
process.exit(0);
