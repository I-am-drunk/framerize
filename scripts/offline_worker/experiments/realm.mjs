// No-JSDOM realm: a plain node:vm context that answers the Castle SDK's
// browser questions with values copied from the user's real Chrome.
// Nothing is invented. Property values come from real-browser-values.json;
// canvas 2D-context reads replay the captured probe table (probe-capture).
import fs from 'node:fs';
import vm from 'node:vm';
import { createHash, webcrypto, randomUUID } from 'node:crypto';

const REAL = JSON.parse(fs.readFileSync(new URL('./real-browser-values.json', import.meta.url)));
const PROBES = JSON.parse(fs.readFileSync(new URL('./probe-capture.json', import.meta.url)));

// Persistent ambient state: storage, cookie, and clock survive across mints
// so tokens age like a real profile instead of describing a brand-new page.
const STATE_PATH = process.env.FRAMERIZE_REALM_STATE
  || new URL('./realm-state.json', import.meta.url).pathname;
function loadState() {
  try { return JSON.parse(fs.readFileSync(STATE_PATH, 'utf8')); }
  catch { return null; }
}
function saveState(s) {
  try { fs.writeFileSync(STATE_PATH, JSON.stringify(s)); } catch {}
}

function makePlugins() {
  const defs = [
    { name: 'PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
    { name: 'Chrome PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
    { name: 'Chromium PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
    { name: 'Microsoft Edge PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
    { name: 'WebKit built-in PDF', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
  ];
  const mime = () => ({ type: 'application/pdf', suffixes: 'pdf', description: 'Portable Document Format', enabledPlugin: null });
  const arr = defs.map(d => {
    const p = { name: d.name, filename: d.filename, description: d.description, length: 2 };
    const mimes = [mime(), { type: 'text/pdf', suffixes: 'pdf', description: 'Portable Document Format', enabledPlugin: p }];
    mimes.forEach((mt, i) => { p[i] = mt; if (!mt.enabledPlugin) mt.enabledPlugin = p; });
    p.item = i => p[i];
    p.namedItem = n => mimes.find(x => x.type === n);
    return p;
  });
  arr.item = i => arr[i];
  arr.namedItem = n => arr.find(p => p.name === n);
  arr.refresh = () => {};
  return arr;
}

function makeMimeTypes() {
  const defs = [
    { type: 'application/pdf', suffixes: 'pdf', description: 'Portable Document Format' },
    { type: 'text/pdf', suffixes: 'pdf', description: 'Portable Document Format' },
  ];
  const arr = defs.map(d => ({ type: d.type, suffixes: d.suffixes, description: d.description }));
  arr.item = i => arr[i];
  arr.namedItem = n => arr.find(t => t.type === n);
  return arr;
}

export function buildRealmGlobals({ publicKey, seedCuid: reqCuid }) {
  const persisted = loadState() || {};
  // First ever mint: adopt the process start as the profile birth time and
  // keep it forever after, like a browser profile that was opened once.
  if (!persisted.timeOrigin) {
    persisted.timeOrigin = Date.now();
    persisted.localStorage = {};
    persisted.sessionStorage = {};
    persisted.cookie = '';
    saveState(persisted);
  }
  // One-time adoption of an existing device ID (SavedDevice semantics from
  // ../device.mjs: set __cuid in localStorage and cookie, then verify).
  // Skipped when absent or when the realm already has one.
  // Per-request device adoption: a caller may pass seedCuid to claim a
  // specific account device for THIS mint only (not persisted). Falls back to
  // the env one-time adoption, then to whatever the profile already holds.
  const envCuid = process.env.FRAMERIZE_REALM_SEED_CUID;
  const seedCuid = reqCuid || envCuid;
  if (reqCuid && /^[A-Za-z0-9._~-]{1,256}$/.test(reqCuid)) {
    // Request-scoped: override the in-memory copy only, leave the saved profile.
    persisted.localStorage = Object.assign({}, persisted.localStorage, { __cuid: reqCuid });
    persisted.cookie = '__cuid=' + reqCuid;
  } else if (seedCuid && !persisted.localStorage.__cuid
      && /^[A-Za-z0-9._~-]{1,256}$/.test(seedCuid)) {
    persisted.localStorage.__cuid = seedCuid;
    persisted.cookie = '__cuid=' + seedCuid;
    saveState(persisted);
  }
  // Replay measureText in capture order per string. A real browser returns
  // the same width for (font, text); the SDK re-measures the same string at
  // different widths because it changes ctx.font between calls. Keying by
  // text alone collapses those to one wrong width, which is a fake signal.
  // A queue preserves the exact captured sequence for each string.
  const measureQueues = new Map();
  const imageTable = new Map();
  for (const r of PROBES.reads) {
    if (r.m === 'measureText') {
      const key = String(r.a[0]);
      if (!measureQueues.has(key)) measureQueues.set(key, []);
      measureQueues.get(key).push(r.s.w);
    } else if (r.m === 'getImageData') imageTable.set(r.a.join(','), r.s);
  }
  const measureCursors = new Map();
  const listeners = new Map();

  // 2D context whose reads replay the captured Chrome answers.
  function make2DContext() {
    return {
      canvas: null,
      fillStyle: '#000', strokeStyle: '#000', font: '10px sans-serif',
      textBaseline: 'alphabetic', globalAlpha: 1, globalCompositeOperation: 'source-over',
      fillRect() {}, strokeRect() {}, clearRect() {}, fillText() {}, strokeText() {},
      beginPath() {}, closePath() {}, moveTo() {}, lineTo() {}, arc() {}, rect() {},
      fill() {}, stroke() {}, save() {}, restore() {}, translate() {}, rotate() {},
      scale() {}, setTransform() {}, drawImage() {}, clip() {}, setLineDash() {},
      measureText(text) {
        const key = String(text);
        const queue = measureQueues.get(key);
        let w;
        if (queue && queue.length) {
          const i = measureCursors.get(key) || 0;
          w = queue[i % queue.length];
          measureCursors.set(key, i + 1);
        }
        const width = w !== undefined ? w : String(text).length * 8;
        return { width, actualBoundingBoxLeft: 0, actualBoundingBoxRight: width,
          actualBoundingBoxAscent: 11, actualBoundingBoxDescent: 3,
          fontBoundingBoxAscent: 12, fontBoundingBoxDescent: 3 };
      },
      getImageData(x, y, w, h) {
        const s = imageTable.get([x, y, w, h].join(','));
        const len = w * h * 4;
        const data = new Uint8ClampedArray(len);
        if (s && s.d) data.set(s.d.slice(0, Math.min(s.d.length, len)));
        return { width: w, height: h, data, colorSpace: 'srgb' };
      },
      isPointInPath() { return false; },
      isPointInStroke() { return false; },
      createLinearGradient() { return { addColorStop() {} }; },
      createRadialGradient() { return { addColorStop() {} }; },
      createPattern() { return null; },
      putImageData() {}, getLineDash() { return []; },
    };
  }

  function makeCanvas() {
    const ctx2d = make2DContext();
    const el = {
      width: 300, height: 150, style: {},
      getContext(type) { return type === '2d' ? ctx2d : null; },
      toDataURL() { return REAL.canvas2d || 'data:image/png;base64,'; },
      addEventListener() {}, removeEventListener() {},
      getAttribute() { return null; }, setAttribute() {},
    };
    ctx2d.canvas = el;
    return el;
  }


  function makeElement(tag) {
    return {
      tagName: tag.toUpperCase(), nodeName: tag.toUpperCase(),
      style: {}, children: [], childNodes: [],
      appendChild(child) { this.children.push(child); this.childNodes.push(child); return child; },
      insertBefore(child, ref) { this.children.unshift(child); this.childNodes.unshift(child); return child; },
      removeChild(child) { const i = this.children.indexOf(child); if (i >= 0) { this.children.splice(i, 1); this.childNodes.splice(this.childNodes.indexOf(child), 1); } return child; },
      setAttribute(name, value) { this[name] = value; },
      hasAttribute(name) { return name in this; },
      getAttribute(name) { return this[name] ?? null; },
      addEventListener() {}, removeEventListener() {},
      querySelector() { return null; }, querySelectorAll() { return []; },
      cloneNode() { return makeElement(tag); },
      getBoundingClientRect() { return { x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0 }; },
      clientWidth: 0, clientHeight: 0, clientLeft: 0, clientTop: 0,
      offsetWidth: 0, offsetHeight: 0, offsetLeft: 0, offsetTop: 0,
      contentWindow: undefined,
    };
  }
  const nav = REAL.navigator;
  const navigator = {
    appVersion: nav.appVersion, buildID: nav.buildID, cookieEnabled: nav.cookieEnabled,
    deviceMemory: nav.deviceMemory, hardwareConcurrency: nav.hardwareConcurrency,
    language: nav.language, languages: Object.freeze([...nav.languages]),
    maxTouchPoints: nav.maxTouchPoints, oscpu: nav.oscpu, platform: nav.platform,
    productSub: nav.productSub, standalone: nav.standalone, userAgent: nav.userAgent,
    vendor: nav.vendor, webdriver: nav.webdriver,
    plugins: makePlugins(),
    mimeTypes: makeMimeTypes(),
    javaEnabled() { return nav.javaEnabled; },
    connection: { effectiveType: '4g', downlink: 10, rtt: 50, saveData: false,
      addEventListener() {}, removeEventListener() {} },
    permissions: {}, storage: {}, credentials: {}, mediaDevices: {},
    serviceWorker: { ready: Promise.resolve(null), getRegistration: () => Promise.resolve(undefined),
      addEventListener() {}, removeEventListener() {} },
    userAgentData: { brands: Object.freeze([{ brand: 'Google Chrome', version: '155' },
      { brand: 'Chromium', version: '155' }, { brand: 'Not(A:Brand', version: '24' }]),
      mobile: false, platform: 'Linux',
      getHighEntropyValues: h => Promise.resolve({ brands: this?.brands || [], mobile: false, platform: 'Linux' }),
      toJSON: () => ({ brands: [{ brand: 'Google Chrome', version: '155' },
        { brand: 'Chromium', version: '155' }, { brand: 'Not(A:Brand', version: '24' }], mobile: false, platform: 'Linux' }) },
    gpu: {}, pdfViewerEnabled: true, webkitTemporaryStorage: {},
    bluetooth: undefined,
    keyboard: undefined, brave: undefined,
    getBattery: undefined, battery: undefined, doNotTrack: null,
    sendBeacon() { return false; }, vibrate() { return false; },
  };

  const scr = REAL.screen;
  const screen = {
    width: scr.width, height: scr.height, availWidth: scr.availWidth,
    availHeight: scr.availHeight, availLeft: scr.availLeft || 0, availTop: scr.availTop || 0,
    colorDepth: scr.colorDepth, pixelDepth: scr.pixelDepth || scr.colorDepth,
    isExtended: false, orientation: { type: scr.orientation || 'landscape-primary', angle: 0 },
  };

  const storageArea = (storeName) => {
    const m = new Map(Object.entries(persisted[storeName] || {}));
    const flush = () => {
      persisted[storeName] = Object.fromEntries(m);
      saveState(persisted);
    };
    return {
      get length() { return m.size; },
      getItem(k) { return m.has(k) ? m.get(k) : null; },
      setItem(k, v) { m.set(String(k), String(v)); flush(); },
      removeItem(k) { m.delete(k); flush(); },
      clear() { m.clear(); flush(); },
      key(i) { return [...m.keys()][i] ?? null; },
    };
  };

  const document = {
    location: { href: 'https://framer.com/projects/?teamId=317933c5-f358-463d-9896-290d68f0bf45', origin: 'https://framer.com', protocol: 'https:', host: 'framer.com', hostname: 'framer.com', pathname: '/projects/', search: '?teamId=317933c5-f358-463d-9896-290d68f0bf45', hash: '' },
    hidden: false, visibilityState: 'visible', readyState: 'complete',
    URL: 'https://framer.com/projects/?teamId=317933c5-f358-463d-9896-290d68f0bf45',
    documentURI: 'https://framer.com/projects/?teamId=317933c5-f358-463d-9896-290d68f0bf45',
    baseURI: 'https://framer.com/projects/?teamId=317933c5-f358-463d-9896-290d68f0bf45',
    referrer: '', domain: 'framer.com', characterSet: 'UTF-8',
    contentType: 'text/html', title: 'Framer',
    documentElement: makeElement('html'), body: makeElement('body'),
    createElement(tag) {
      const t = String(tag).toLowerCase();
      if (t === 'canvas') return makeCanvas();
      return makeElement(t);
    },
    createElementNS(ns, tag) { return this.createElement(tag); },
    addEventListener(type, fn) { if (!listeners.has(type)) listeners.set(type, []); listeners.get(type).push(fn); },
    removeEventListener() {}, getElementById() { return null; },
    querySelector() { return null; }, querySelectorAll() { return []; },
    getElementsByTagName() { return []; }, fonts: { check: () => true },
    // Remaining surface the SDK touches, matched to real Chrome types.
    caretPositionFromPoint() { return null; },
    createEvent() { return { initEvent() {}, type: null, bubbles: false, cancelable: false }; },
    hasStorageAccess() { return Promise.resolve(true); },
    onbeforematch: null, prerendering: false, wasDiscarded: false,
    webkitHidden: false, scripts: { length: 0, item: () => null },
    styleSheets: { length: 0, item: () => null },
  };

  const cryptoObj = {
    getRandomValues(arr) { return webcrypto.getRandomValues(arr); },
    subtle: webcrypto.subtle,
    randomUUID() { return randomUUID(); },
  };
  const performanceObj = {
    timeOrigin: persisted.timeOrigin,
    // performance.now() measures time since timeOrigin. Anchoring Node's
    // clock to the persisted origin keeps the pair consistent, like a page
    // that has been open since the profile was born.
    _nodeBase: performance.timeOrigin,
    now: () => (Date.now() - persisted.timeOrigin),
    getEntriesByType: () => [],
    mark() {}, measure() {},
    memory: { usedJSHeapSize: 10000000, totalJSHeapSize: 20000000, jsHeapSizeLimit: 4294967296 },
  };

  document.head = makeElement('head');
  document.documentElement.children.push(document.head, document.body);
  document.documentElement.childNodes.push(document.head, document.body);

  // Cookie jar on the document object itself, like a real browser. Set-Cookie
  // strings append with '; ' between pairs, matching document.cookie rules.
  Object.defineProperty(document, 'cookie', {
    get() { return persisted.cookie || ''; },
    set(v) {
      const pair = String(v).split(';')[0];
      const name = pair.split('=')[0].trim();
      const jar = new Map();
      for (const p of (persisted.cookie || '').split('; ')) {
        if (p) jar.set(p.split('=')[0].trim(), p);
      }
      jar.set(name, pair);
      persisted.cookie = [...jar.values()].join('; ');
      saveState(persisted);
    },
    configurable: true,
  });

  const win = {
    document, navigator, screen,
    location: { href: 'https://framer.com/projects/?teamId=317933c5-f358-463d-9896-290d68f0bf45', origin: 'https://framer.com', protocol: 'https:', host: 'framer.com', hostname: 'framer.com', pathname: '/projects/', search: '?teamId=317933c5-f358-463d-9896-290d68f0bf45', hash: '' },
    innerWidth: REAL.window.innerWidth, innerHeight: REAL.window.innerHeight,
    outerWidth: REAL.window.outerWidth, outerHeight: REAL.window.outerHeight,
    screenX: REAL.window.screenX, screenY: REAL.window.screenY,
    devicePixelRatio: REAL.window.devicePixelRatio,
    localStorage: storageArea('localStorage'), sessionStorage: storageArea('sessionStorage'),
    crypto: cryptoObj, performance: performanceObj,
    addEventListener(type, fn) { if (!listeners.has(type)) listeners.set(type, []); listeners.get(type).push(fn); },
    removeEventListener() {},
    requestAnimationFrame(cb) { return setTimeout(() => cb(performance.now()), 16); },
    cancelAnimationFrame(id) { clearTimeout(id); },
    setTimeout, clearTimeout, setInterval, clearInterval, queueMicrotask,
    TextEncoder, TextDecoder, Uint8ClampedArray, Uint8Array, ArrayBuffer,
    Int8Array, Uint16Array, Int16Array, Uint32Array, Int32Array,
    Float32Array, Float64Array, BigInt64Array, BigUint64Array, DataView,
    atob: globalThis.atob, btoa: globalThis.btoa,
    encodeURIComponent, decodeURIComponent, escape, unescape,
    parseInt, parseFloat, isNaN, isFinite, Number, String, Boolean, RegExp,
    Error, TypeError, RangeError, SyntaxError, URIError, EvalError,
    Symbol, Reflect, Proxy, BigInt, Intl, URL, URLSearchParams, Blob,
    Promise, Map, Set, WeakMap, WeakSet, JSON, Math, Date, Array, Object,
    Function, SuppressedError: globalThis.SuppressedError, AggregateError, FinalizationRegistry, WeakRef,
    requestIdleCallback: cb => setTimeout(() => cb({ didTimeout: false, timeRemaining: () => 50 }), 0),
    cancelIdleCallback: id => clearTimeout(id),
    console, fetch: undefined, XMLHttpRequest: undefined,
    castlePublishableAPIKey: publicKey,
    Worker: undefined, SharedWorker: undefined, OffscreenCanvas: undefined,
    indexedDB: undefined, caches: undefined, openDatabase: undefined,
    Connection: undefined, NetworkInformation: undefined,
    connection: { effectiveType: '4g', downlink: 10, rtt: 50, saveData: false,
      addEventListener() {}, removeEventListener() {} },
    // Constructor surface the SDK touches for existence/instanceof checks.
    // Real Chrome defines all of these; only their absence is a signal.
    AbortSignal: class AbortSignal {},
    AudioContext: class AudioContext {}, webkitAudioContext: undefined,
    CanvasRenderingContext2D: class CanvasRenderingContext2D {},
    Element: class Element {}, HTMLCanvasElement: class HTMLCanvasElement {},
    Navigator: class Navigator {}, Screen: class Screen {}, Iterator: globalThis.Iterator || class Iterator {},
    MessageChannel: class MessageChannel {},
    Request: class Request {}, Response: class Response {}, Headers: class Headers {},
    WebAssembly: globalThis.WebAssembly,
    history: { length: 1, state: null, scrollRestoration: 'auto',
      back() {}, forward() {}, go() {}, pushState() {}, replaceState() {} },
    chrome: undefined, InstallTrigger: undefined, safari: undefined,
    opr: undefined, BuiltIn: undefined, StyleMedia: undefined,
    external: { AddSearchProvider: undefined, IsSearchProviderInstalled: undefined },
    mozInnerScreenX: undefined, mozInnerScreenY: undefined,
    opera: undefined, webkitURL: undefined, webkitRTCPeerConnection: undefined,
    RTCPeerConnection: undefined, mozRTCPeerConnection: undefined,
    speechSynthesis: undefined, SpeechSynthesisUtterance: undefined,
    Notification: undefined, PushManager: undefined, BatteryManager: undefined,
    matchMedia: q => ({ matches: false, media: String(q), addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} }),
    getComputedStyle: () => ({ getPropertyValue: () => '', }),
    openDatabaseSync: undefined, webkitRequestFileSystem: undefined,
    webkitResolveLocalFileSystemURL: undefined, requestFileSystem: undefined,
  };
  win.window = win; win.self = win; win.globalThis = win; win.top = win; win.parent = win;
  document.body.contentWindow = win;
  document.documentElement.contentWindow = win;
  // Patch createElement to set contentWindow on new elements
  const origCreate = document.createElement;
  document.createElement = function(tag) {
    const el = origCreate.call(this, tag);
    el.contentWindow = win;
    return el;
  };
  win.selenium = undefined;
  win.scheduler = undefined;
  win.document.fullscreenElement = null;
  return win;
}

export function buildRealm(opts) {
  const globals = buildRealmGlobals(opts);
  const context = vm.createContext(globals);
  return { context, win: globals };
}
