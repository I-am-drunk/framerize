// Experimental direct EGL binding. This does not expose a WebGL API.
import koffi from 'koffi';

const egl = koffi.load('libEGL.so.1');
const gles = koffi.load('libGLESv2.so.2');
const displayFor = egl.func('void *eglGetPlatformDisplay(uint32_t, void *, const intptr_t *)');
const initialize = egl.func('uint32_t eglInitialize(void *, _Out_ int *, _Out_ int *)');
const bind = egl.func('uint32_t eglBindAPI(uint32_t)');
const choose = egl.func('uint32_t eglChooseConfig(void *, const int *, _Out_ void **, int, _Out_ int *)');
const surfaceFor = egl.func('void *eglCreatePbufferSurface(void *, void *, const int *)');
const contextFor = egl.func('void *eglCreateContext(void *, void *, void *, const int *)');
const current = egl.func('uint32_t eglMakeCurrent(void *, void *, void *, void *)');
const destroyContext = egl.func('uint32_t eglDestroyContext(void *, void *)');
const destroySurface = egl.func('uint32_t eglDestroySurface(void *, void *)');
const terminate = egl.func('uint32_t eglTerminate(void *)');
const release = egl.func('uint32_t eglReleaseThread()');
const error = egl.func('uint32_t eglGetError()');
const stringFor = gles.func('const char *glGetString(uint32_t)');
const glError = gles.func('uint32_t glGetError()');
let owner = null;
const attrs = (...values) => [...values, 0x3038];
const requireResult = (value, phase) => {
  if (!value) throw new Error(`${phase}: EGL ${error().toString(16)}`);
  return value;
};

export class NativeContext {
  #display; #surface; #context; #initialized = false; #closed = false;
  constructor() {
    if (owner) throw new Error('native context already owned');
    owner = this;
    try {
      this.#display = requireResult(displayFor(0x31dd, null, null), 'display');
      const major = [0], minor = [0];
      requireResult(initialize(this.#display, major, minor), 'initialize');
      this.#initialized = true;
      requireResult(bind(0x30a0), 'bind');
      const configs = [null], count = [0];
      requireResult(choose(this.#display, attrs(0x3033, 1, 0x3040, 4,
        0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3021, 8), configs, 1, count), 'choose');
      if (count[0] !== 1) throw new Error('no RGBA8 configuration');
      this.#surface = requireResult(surfaceFor(this.#display, configs[0],
        attrs(0x3057, 64, 0x3056, 64)), 'surface');
      this.#context = requireResult(contextFor(this.#display, configs[0], null,
        attrs(0x3098, 2)), 'context');
      requireResult(current(this.#display, this.#surface, this.#surface,
        this.#context), 'current');
      this.version = Object.freeze([major[0], minor[0]]);
    } catch (error) { this.close(); throw error; }
  }
  query() {
    if (this.#closed) throw new Error('context closed');
    requireResult(current(this.#display, this.#surface, this.#surface,
      this.#context), 'current');
    const values = Object.fromEntries(Object.entries({ vendor: 0x1f00,
      renderer: 0x1f01, version: 0x1f02, extensions: 0x1f03 })
      .map(([key, value]) => [key, stringFor(value)]));
    if (glError() || Object.values(values).some(x => typeof x !== 'string' || !x))
      throw new Error('native query failed');
    return values;
  }
  close() {
    if (this.#closed) return;
    const failures = [];
    if (this.#initialized) {
      if (!current(this.#display, null, null, null)) failures.push('release current');
      if (this.#context && !destroyContext(this.#display, this.#context)) failures.push('context');
      if (this.#surface && !destroySurface(this.#display, this.#surface)) failures.push('surface');
      if (!terminate(this.#display)) failures.push('display');
    }
    if (!release()) failures.push('thread');
    this.#closed = true;
    owner = null;
    if (failures.length) throw new Error('EGL cleanup: ' + failures.join(', '));
  }
}
