// Production-compatible window factory backed by the no-JSDOM realm.
// Satisfies every check OfflineCastleMinter performs on dom.window and
// dom.getInternalVMContext, so it is a drop-in for createOfflineWindow.
import { installSharedWorkers } from './shared_workers.mjs';
import { installDedicatedWorkers } from './dedicated_workers.mjs';
import { installWebGL } from './canvas_webgl.mjs';
import { buildRealmGlobals } from './realm_globals.mjs';
import vm from 'node:vm';
import { Blob } from 'node:buffer';

// Minimal ErrorEvent (Node 20 has Event/EventTarget/MessageEvent but not
// ErrorEvent). Only what the worker installers construct.
class ErrorEvent extends Event {
  constructor(type, options = {}) { super(type, options); this.message = options.message ?? ''; this.error = options.error ?? null; }
}

const SHARED_WORKER_SHA256 = 'cb1716096c87eff9d6f2296bbac1302c0b3c58e404226b15cd45cd223c4addab';
const DEDICATED_WORKER_SHA256 = [
  'f9bbca7372aaea93762997eb78a3a3c3198132b22bcb787754b64059087ad4ff',
  '7fbadc72ebc1e56a4274d005ec9ce05ad2435bbcc2406f8640712bbdd0987176',
];

export function createRealmWindow(publicKey, cuid, targetUrl) {
  const globals = buildRealmGlobals({ publicKey, cuid, targetUrl });
  const context = vm.createContext(globals);
  // Patch Function.prototype.toString inside the realm so every function the
  // realm defined reports the browser's native-source form. The replacement
  // lives on the prototype, not as an own property on each function, so
  // hasOwnProperty('toString') is false and Function.prototype.toString.call(fn)
  // returns the same native string a real browser gives.
  vm.runInContext(`(() => {
    const sources = new WeakMap();
    const orig = Function.prototype.toString;
    const nameOf = fn => {
      const n = fn.name;
      return 'function ' + (n || '') + '() { [native code] }';
    };
    Function.prototype.toString = function toString() {
      if (sources.has(this)) return sources.get(this);
      let out;
      try { out = orig.call(this); } catch (e) { out = null; }
      // A genuine V8 builtin already carries the native marker; pass it through.
      if (out && out.includes('[native code]')) return out;
      const form = nameOf(this);
      sources.set(this, form);
      return form;
    };
    // The patch itself must read as native too, else Function.prototype.toString
    // .toString() leaks this function's source. Register it before any caller.
    sources.set(Function.prototype.toString, 'function toString() { [native code] }');
  })();`, context);
  const win = globals;
  if (typeof win.TextEncoder === 'undefined') win.TextEncoder = TextEncoder;
  win.requestAnimationFrame ??= cb => setTimeout(() => cb(performance.now()), 16);
  win.cancelAnimationFrame ??= id => clearTimeout(id);
  if (!win.document.hidden) win.document.hidden = false;
  win.document.visibilityState ??= 'visible';

  // Event primitives the worker installers construct/extend.
  win.EventTarget ??= EventTarget;
  win.Event ??= Event;
  win.MessageEvent ??= MessageEvent;
  win.ErrorEvent ??= ErrorEvent;
  win.DOMException ??= DOMException;
  win.Blob ??= Blob;

  // Install real worker runtimes so the v3 token path can complete and
  // wrapper.a returns the wire string (not the raw collection array).
  let sharedWorkers = null, dedicatedWorkers = null;
  try {
    sharedWorkers = installSharedWorkers(win, SHARED_WORKER_SHA256);
    dedicatedWorkers = installDedicatedWorkers(win, DEDICATED_WORKER_SHA256);
  } catch (e) {
    // Worker installation is best-effort here; leave globals for diagnostics.
    win.__workerInstallError = String(e && e.message || e);
  }

  const close = async () => {
    const errors = [];
    if (dedicatedWorkers) { try { await dedicatedWorkers.close(); } catch (e) { errors.push('dedicated'); } }
    if (sharedWorkers) { try { await sharedWorkers.close(); } catch (e) { errors.push('shared'); } }
    return { sharedWorkers, dedicatedWorkers, errors };
  };
  return { window: win, getInternalVMContext: () => context, close,
    workerStats: () => ({
      shared: sharedWorkers ? sharedWorkers.stats() : null,
      dedicated: dedicatedWorkers ? dedicatedWorkers.stats() : null,
      dedicatedReplies: dedicatedWorkers ? dedicatedWorkers.replyStats() : null,
    }) };
}
