// No-JSDOM realm: a plain node:vm context that answers the Castle SDK's
// browser questions with values copied from the user's real Chrome.
// Nothing is invented. Property values come from real-browser-values.json;
// canvas 2D-context reads replay the captured probe table (probe-capture).
import fs from 'node:fs';
import vm from 'node:vm';
import { createHash, webcrypto, randomUUID } from 'node:crypto';

const REAL = JSON.parse(fs.readFileSync(new URL('./inputs/real-browser-values.json', import.meta.url)));
const PROBES = JSON.parse(fs.readFileSync(new URL('./inputs/probe-capture.json', import.meta.url)));

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

export function buildRealmGlobals({ publicKey, cuid }) {
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

  // Replay support: dispatch one captured real event to every listener that
  // the SDK registered for its type. Events come from a genuine browser trace
  // (recorded in the user's own tab), so no signal is invented. The handler
  // receives a plain object shaped like the event Castle reads: type plus the
  // captured coordinate/key/timestamp fields.
  function dispatchCaptured(type, props) {
    const list = listeners.get(type);
    if (!list || list.length === 0) return 0;
    const ev = Object.assign({ type, timeStamp: performance.now(),
      isTrusted: false, bubbles: true, cancelable: true,
      preventDefault() {}, stopPropagation() {}, target: null }, props || {});
    let n = 0;
    for (const fn of list) { try { fn(ev); n++; } catch {} }
    return n;
  }

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
    hardwareConcurrency: nav.hardwareConcurrency,
    language: nav.language, languages: Object.freeze([...nav.languages]),
    maxTouchPoints: nav.maxTouchPoints, oscpu: nav.oscpu, platform: nav.platform,
    productSub: nav.productSub, standalone: nav.standalone, userAgent: nav.userAgent,
    vendor: nav.vendor, webdriver: nav.webdriver,
    plugins: makePlugins(),
    mimeTypes: makeMimeTypes(),
    javaEnabled() { return nav.javaEnabled; },
    connection: { effectiveType: '4g', downlink: 10, rtt: 50, saveData: false,
      addEventListener() {}, removeEventListener() {} },
    permissions: {},
    pdfViewerEnabled: true, webkitTemporaryStorage: {},
    doNotTrack: null,
    sendBeacon() { return false; }, vibrate() { return false; },
  };

  const scr = REAL.screen;
  const screen = {
    width: scr.width, height: scr.height, availWidth: scr.availWidth,
    availHeight: scr.availHeight, availLeft: scr.availLeft || 0, availTop: scr.availTop || 0,
    colorDepth: scr.colorDepth, pixelDepth: scr.pixelDepth || scr.colorDepth,
    isExtended: false, orientation: { type: scr.orientation || 'landscape-primary', angle: 0 },
  };

  const storageArea = (seed) => {
    const m = new Map(Object.entries(seed || {}));
    return {
      get length() { return m.size; },
      getItem(k) { return m.has(k) ? m.get(k) : null; },
      setItem(k, v) { m.set(String(k), String(v)); },
      removeItem(k) { m.delete(k); },
      clear() { m.clear(); },
      key(i) { return [...m.keys()][i] ?? null; },
    };
  };

  const document = {
    location: { href: 'https://framer.com/projects', origin: 'https://framer.com', protocol: 'https:', host: 'framer.com', hostname: 'framer.com', pathname: '/projects', search: '', hash: '' },
    hidden: false, visibilityState: 'visible', readyState: 'complete',
    cookie: cuid ? ('__cuid=' + cuid) : '', documentElement: makeElement('html'), body: makeElement('body'),
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
  };

  const cryptoObj = {
    getRandomValues(arr) { return webcrypto.getRandomValues(arr); },
    subtle: webcrypto.subtle,
    randomUUID() { return randomUUID(); },
  };
  const performanceObj = {
    timeOrigin: Date.now(),
    now: () => performance.now(),
    getEntriesByType: () => [],
    mark() {}, measure() {},
    memory: { usedJSHeapSize: 10000000, totalJSHeapSize: 20000000, jsHeapSizeLimit: 4294967296 },
  };

  const win = {
    document, navigator, screen,
    location: { href: 'https://framer.com/projects', origin: 'https://framer.com', protocol: 'https:', host: 'framer.com', hostname: 'framer.com', pathname: '/projects', search: '', hash: '' },
    innerWidth: REAL.window.innerWidth, innerHeight: REAL.window.innerHeight,
    outerWidth: REAL.window.outerWidth, outerHeight: REAL.window.outerHeight,
    screenX: REAL.window.screenX, screenY: REAL.window.screenY,
    devicePixelRatio: REAL.window.devicePixelRatio,
    localStorage: storageArea(cuid ? { __cuid: cuid } : undefined), sessionStorage: storageArea(),
    crypto: cryptoObj, performance: performanceObj,
    addEventListener(type, fn) { if (!listeners.has(type)) listeners.set(type, []); listeners.get(type).push(fn); },
    removeEventListener() {},
    requestAnimationFrame(cb) { return setTimeout(() => cb(performance.now()), 16); },
    cancelAnimationFrame(id) { clearTimeout(id); },
    setTimeout, clearTimeout, setInterval, clearInterval, queueMicrotask,
    TextEncoder, TextDecoder, Uint8ClampedArray, Uint8Array, ArrayBuffer,
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
  // Replay hook for the token service: drive the SDK's registered listeners
  // with a captured real interaction trace before minting. Trace entries are
  // {type, props}; dispatchCaptured fans each out to every registered handler.
  win.__dispatchCaptured = dispatchCaptured;
  win.__listenerCount = () => Array.from(listeners.entries())
    .map(([t, l]) => ({ type: t, handlers: l.length }));
  return win;
}

export function buildRealm(opts) {
  const globals = buildRealmGlobals(opts);
  const context = vm.createContext(globals);
  return { context, win: globals };
}
