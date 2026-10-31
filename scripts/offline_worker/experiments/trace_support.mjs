// Prototype-level API access tracer for the pinned Castle SDK under JSDOM.
// Wraps getter results on Window/Navigator/Screen/Performance prototypes so
// every object the SDK receives is a recording proxy. Observation-only:
// proxies change object identity, so this never runs a real mint.
export const TRACE_LIMIT = 200000;

export function makeTraceState() {
  return { records: [], truncated: false };
}

export function record(state, path, kind, shape) {
  if (state.records.length >= TRACE_LIMIT) { state.truncated = true; return; }
  state.records.push(path + '|' + kind + '|' + shape);
}

function shapeOf(value) {
  if (value === null) return 'null';
  const t = typeof value;
  if (t !== 'object') return t;
  if (Array.isArray(value)) return 'array:' + value.length;
  const tag = value.constructor && value.constructor.name;
  return 'object:' + (tag || 'anon');
}

const RAW = Symbol('traceRaw');

export function makeWrapper(state) {
  const cache = new WeakMap();
  function wrap(value, path) {
    if (value === null || (typeof value !== 'object' && typeof value !== 'function'))
      return value;
    if (cache.has(value)) return cache.get(value);
    const proxy = new Proxy(value, {
      get(target, prop) {
        if (prop === RAW) return value;
        if (prop === Symbol.toPrimitive) return target[Symbol.toPrimitive];
        if (typeof prop === 'symbol') return Reflect.get(target, prop);
        let result;
        try { result = Reflect.get(target, prop, target); }
        catch { return undefined; }
        const child = path + '.' + String(prop);
        record(state, child, 'get', shapeOf(result));
        return wrap(result, child);
      },
      apply(target, thisArg, args) {
        const plain = args.map(a => (a && a[RAW]) ? a[RAW] : a);
        const self = (thisArg && thisArg[RAW]) ? thisArg[RAW] : thisArg;
        record(state, path + '()', 'call', 'args:' + args.length);
        let out;
        try { out = Reflect.apply(target, self, plain); }
        catch (e) { record(state, path + '()', 'throw', String(e && e.message)); throw e; }
        record(state, path + '()', 'return', shapeOf(out));
        return wrap(out, path + '()');
      },
    });
    cache.set(value, proxy);
    return proxy;
  }
  return wrap;
}

// Redefine the redefinable getter properties on the JSDOM Window instance
// (own accessor properties win over prototype ones). Locked properties
// (document, location) keep their real values, which are then wrapped the
// first time they are returned through a wrapped parent.
export function installOnWindow(win, wrap) {
  for (const key of ['navigator', 'screen', 'performance', 'history',
      'localStorage', 'sessionStorage', 'caches', 'crypto', 'indexedDB']) {
    const desc = Object.getOwnPropertyDescriptor(win, key);
    if (!desc || !desc.configurable || !desc.get) continue;
    Object.defineProperty(win, key, {
      configurable: true,
      get() { return wrap(desc.get.call(this), 'window.' + key); },
    });
  }
  // Data properties the SDK commonly reads.
  for (const key of ['devicePixelRatio', 'innerWidth', 'innerHeight',
      'outerWidth', 'outerHeight', 'screenX', 'screenY']) {
    const desc = Object.getOwnPropertyDescriptor(win, key);
    if (!desc || !desc.configurable || !('value' in desc)) continue;
    const original = desc.value;
    Object.defineProperty(win, key, {
      configurable: true,
      get() { record0('window.' + key); return original; },
    });
  }
  let state0;
  function record0(path) { if (state0) record(state0, path, 'get', 'number'); }
  return s => { state0 = s; };
}
