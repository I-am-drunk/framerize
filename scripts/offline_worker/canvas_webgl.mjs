// Bounded HTML canvas bridge to a supplied, real native WebGL implementation.
let active = false;
export function installWebGL(window, createContext, cleanup, activeContexts) {
  const prototype = window.HTMLCanvasElement.prototype;
  const original = Object.getOwnPropertyDescriptor(prototype, 'getContext');
  if (typeof original?.value !== 'function' || typeof createContext !== 'function'
      || typeof cleanup !== 'function' || typeof activeContexts !== 'function')
    throw new Error('graphics input');
  if (active || activeContexts() !== 0) throw new Error('graphics owner active');
  const modes = new WeakMap();
  let closed = false, pixels = 0, created = 0, refused = 0;
  function getContext(kind, options) {
    if (closed) throw new Error('graphics closed');
    const type = String(kind);
    const mode = type === 'experimental-webgl' ? 'webgl' : type;
    const prior = modes.get(this);
    if (prior) {
      if (prior.mode !== mode) return null;
      if (mode === 'webgl' && (prior.width !== this.width || prior.height !== this.height)) {
        const width = this.width, height = this.height;
        const next = pixels - prior.width * prior.height + width * height;
        if (width < 1 || height < 1 || width > 1024 || height > 1024 || next > 1048576) {
          refused++; throw new Error('graphics resource limit');
        }
        prior.context.getExtension('STACKGL_resize_drawingbuffer').resize(width, height);
        Object.assign(prior, { width, height }); pixels = next;
      }
      return prior.context;
    }
    let context;
    if (mode === 'webgl') {
      const width = this.width, height = this.height;
      const count = activeContexts();
      if (!Number.isSafeInteger(count) || count < 0 || count >= 2 || created >= 8
          || width < 1 || height < 1 || width > 1024 || height > 1024
          || pixels + width * height > 1048576) {
        refused++; throw new Error('graphics resource limit');
      }
      context = createContext(width, height, options);
      if (context) {
        context.canvas = this;
        created++; pixels += width * height;
      }
    } else {
      context = original.value.call(this, type, options);
    }
    if (context) modes.set(this, { mode, context, width: this.width, height: this.height });
    return context;
  }
  Object.defineProperty(prototype, 'getContext', { ...original, value: getContext });
  active = true;
  let result;
  return {
    stats: () => ({ created, refused, pixels, closed }),
    close() {
      if (closed) return result;
      closed = true;
      let destroyed = 0, errors = 0;
      // The native registry owns teardown. A strong JS set would keep discarded
      // canvases alive. Native cleanup also reports failures from GC disposal.
      try {
        if (cleanup() === true) destroyed = created;
        else errors++;
      } catch { errors++; }
      if (prototype.getContext === getContext) {
        Object.defineProperty(prototype, 'getContext', original);
      } else errors++;
      if (!errors) active = false;
      result = Object.freeze({ created, destroyed, refused, errors });
      return result;
    },
  };
}
