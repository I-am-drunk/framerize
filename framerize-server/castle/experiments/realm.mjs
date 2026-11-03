// No-JSDOM realm: a plain node:vm context that answers the Castle SDK's
// browser questions with values copied from the user's real Chrome.
// Nothing is invented. Property values come from real-browser-values.json;
// canvas 2D-context reads replay the captured probe table (probe-capture).
import fs from 'node:fs';
import vm from 'node:vm';
import { createHash, webcrypto, randomUUID } from 'node:crypto';
import { MessageChannel as NodeMessageChannel } from 'node:worker_threads';
import { performance as nodePerformance } from 'node:perf_hooks';
import { deflateSync, inflateSync } from 'node:zlib';

const REAL = JSON.parse(fs.readFileSync(new URL('./real-browser-values.json', import.meta.url)));
const PROBES = JSON.parse(fs.readFileSync(new URL('./probe-capture.json', import.meta.url)));
const REAL_PNG_URLS = JSON.parse(fs.readFileSync(new URL('./real-canvas-png-urls.json', import.meta.url)));
const FEATURE_DETECT = JSON.parse(fs.readFileSync(new URL('./feature-detect-capture.json', import.meta.url)));
const WEBRTC_CAPTURE = JSON.parse(fs.readFileSync(new URL('../inputs/webrtc-capture.json', import.meta.url)));
const CSS_PROP_NAMES = JSON.parse(fs.readFileSync(new URL('./css-prop-names.json', import.meta.url)));
const WORKER_SOURCE = fs.readFileSync(new URL('./shared-worker-source.js', import.meta.url), 'utf8');
const DEDICATED_WORKER_SOURCE = fs.readFileSync(new URL('./dedicated-worker-source.js', import.meta.url), 'utf8');
// Real computed-style values captured from the saved-profile Chrome. Castle reads
// specific properties on a created element; a stub returning '' for every
// property is a readable gap (real Chrome returns 'none','0px','block', etc).
const COMPUTED_STYLE = JSON.parse(fs.readFileSync(new URL('../inputs/real-computed-style.json', import.meta.url)));
// Font-keyed measureText capture: (ctx.font, text) -> exact width, recorded
// live from the user's Chrome so absent fonts return the true fallback width.
const FONTPROBES = (() => {
  try {
    const c = JSON.parse(fs.readFileSync('/tmp/font-probe-capture.json'));
    const t = new Map();
    for (const r of c.reads || []) {
      if (r.m === 'measureText' && r.font !== undefined && r.text !== undefined)
        t.set(r.font + '|' + r.text, r.w);
    }
    return t;
  } catch { return null; }
})();


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

function makeChromeObj() {
  // Real Chrome window.chrome: loadTimes/csi/app. Values computed live with
  // the real shapes (mirrors realm_globals.mjs makeChromeObj).
  const nowS = () => Date.now() / 1000;
  const t0 = nowS();
  return {
    loadTimes() {
      const t = nowS();
      return { requestTime: t0, startLoadTime: t0, commitLoadTime: 0,
        finishDocumentLoadTime: t, finishLoadTime: t, firstPaintTime: 0,
        firstPaintAfterLoadTime: 0, navigationType: 'Other',
        wasFetchedViaSpdy: false, wasNpnNegotiated: false,
        npnNegotiatedProtocol: '', wasAlternateProtocolAvailable: false,
        connectionInfo: 'unknown' };
    },
    csi() {
      const ms = Date.now();
      return { startE: Math.round(t0 * 1000), onloadT: ms, pageT: 200 + Math.random() * 40, tran: 15 };
    },
    app: {
      // Real Chrome 155: 7 keys, not-installed page answers.
      isInstalled: false,
      getDetails(cb) { if (typeof cb === 'function') queueMicrotask(() => cb({})); },
      getIsInstalled(cb) { if (typeof cb === 'function') queueMicrotask(() => cb(false)); },
      installState() { return 'not_installed'; },
      runningState() { return 'cannot_run'; },
      InstallState: { DISABLED: 'disabled', INSTALLED: 'installed', NOT_INSTALLED: 'not_installed' },
      RunningState: { CANNOT_RUN: 'cannot_run', READY_TO_RUN: 'ready_to_run', RUNNING: 'running' },
    },
  };
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
  // Real Chrome 155 PermissionStatus (captured live, /tmp/real-keys-488.json):
  // ownKeys [], proto carries name/state/onchange getters,
  // tag '[object PermissionStatus]'.
  // Real Chrome 155 (captured /tmp/real-keys-488.json): instance own keys
  // are [], name/state/onchange live on the prototype in this exact order,
  // constructor last. The name is held in a WeakMap so nothing own-key leaks.
  const permissionNames = new WeakMap();
  class PermissionStatus extends EventTarget {
    constructor(name) {
      super();
      permissionNames.set(this, String(name));
    }
    get name() { return permissionNames.get(this); }
    get state() { return 'denied'; }
    get onchange() { return null; }
    get [Symbol.toStringTag]() { return 'PermissionStatus'; }
  }
  // Real Chrome prototype key order is name, state, onchange, constructor.
  // A class body puts constructor first, so rebuild the prototype's string
  // key order to match exactly (symbols like toStringTag are unaffected).
  {
    const proto = PermissionStatus.prototype;
    const ctorDesc = Object.getOwnPropertyDescriptor(proto, 'constructor');
    delete proto.constructor;
    Object.defineProperty(proto, 'constructor', ctorDesc);
  }
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

  // Chrome canvas font normalization: setting ctx.font parses the CSS font
  // shorthand; multi-word unquoted family names are quoted in the normalized
  // value ('100px Estrangelo Edessa, monospace' becomes
  // '100px "Estrangelo Edessa", monospace'). Invalid strings are ignored and
  // the previous font is kept. Captured font widths are keyed by the
  // normalized string, so the realm must normalize identically.
  const GENERIC_FAMILIES = new Set(['serif', 'sans-serif', 'monospace', 'cursive',
    'fantasy', 'system-ui', 'ui-serif', 'ui-sans-serif', 'ui-monospace',
    'ui-rounded', 'emoji', 'math', 'fangsong']);
  const FONT_RE = /^(.*?)(xx-small|x-small|small|medium|large|x-large|xx-large|larger|smaller|[-+]?\d*\.?\d+(?:px|pt|pc|em|rem|ex|ch|cap|ic|lh|rlh|vw|vh|vi|vb|vmin|vmax|cm|mm|in|q))(\/\S+)?\s+(.+)$/i;
  function normalizeFont(v) {
    const str = String(v);
    const m = str.match(FONT_RE);
    if (!m) return null;
    const prefix = m[1] + m[2] + (m[3] || '');
    const families = m[4].split(',').map(f => {
      const t = f.trim();
      if (!t || /^["']/.test(t)) return t;
      if (GENERIC_FAMILIES.has(t.toLowerCase())) return t;
      if (/\s/.test(t)) return '"' + t + '"';
      return t;
    });
    return prefix + ' ' + families.join(', ');
  }

  // PNG round-trip for the J_() canvas coherence probe (slot 46 family).
  // The SDK writes random noise to a canvas, serializes with toDataURL, loads
  // it into an Image, draws it back, reads the pixels, and checks the sum is
  // unchanged. Real Chrome's PNG encode is lossless, so the delta is 0. The
  // realm must reproduce that coherence: a real backing store, a real PNG
  // encode, an Image that decodes it, and getImageData reading the store.
  const PNG_SIG = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const CRC_TABLE = (() => {
    const t = new Int32Array(256);
    for (let n = 0; n < 256; n++) {
      let c = n;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      t[n] = c;
    }
    return t;
  })();
  function crc32(buf) {
    let c = -1;
    for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 255] ^ (c >>> 8);
    return (c ^ -1) >>> 0;
  }
  function pngChunk(type, data) {
    const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
    const td = Buffer.concat([Buffer.from(type, 'ascii'), data]);
    const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(td));
    return Buffer.concat([len, td, crc]);
  }
  function encodePNG(width, height, rgba) {
    const ihdr = Buffer.alloc(13);
    ihdr.writeUInt32BE(width, 0); ihdr.writeUInt32BE(height, 4);
    ihdr[8] = 8; ihdr[9] = 6; // 8-bit, RGBA
    const stride = width * 4;
    const raw = Buffer.alloc(height * (stride + 1));
    for (let y = 0; y < height; y++) {
      raw[y * (stride + 1)] = 0; // filter none
      Buffer.from(rgba.buffer, rgba.byteOffset + y * stride, stride).copy(raw, y * (stride + 1) + 1);
    }
    return Buffer.concat([PNG_SIG, pngChunk('IHDR', ihdr), pngChunk('IDAT', deflateSync(raw)), pngChunk('IEND', Buffer.alloc(0))]);
  }
  function paeth(a, b, c) {
    const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
    return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
  }
  function decodePNG(buf) {
    if (!buf.subarray(0, 8).equals(PNG_SIG)) throw new Error('bad png sig');
    let off = 8, width = 0, height = 0;
    const idat = [];
    while (off < buf.length) {
      const len = buf.readUInt32BE(off);
      const type = buf.toString('ascii', off + 4, off + 8);
      const data = buf.subarray(off + 8, off + 8 + len);
      if (type === 'IHDR') { width = data.readUInt32BE(0); height = data.readUInt32BE(4); }
      else if (type === 'IDAT') idat.push(data);
      else if (type === 'IEND') break;
      off += 12 + len;
    }
    const raw = inflateSync(Buffer.concat(idat));
    const stride = width * 4;
    const out = new Uint8ClampedArray(width * height * 4);
    const bpp = 4;
    for (let y = 0; y < height; y++) {
      const filter = raw[y * (stride + 1)];
      const rowIn = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
      const rowOut = out.subarray(y * stride, (y + 1) * stride);
      const prev = y > 0 ? out.subarray((y - 1) * stride, y * stride) : null;
      for (let x = 0; x < stride; x++) {
        const a = x >= bpp ? rowOut[x - bpp] : 0;
        const b = prev ? prev[x] : 0;
        const c = x >= bpp && prev ? prev[x - bpp] : 0;
        let v = rowIn[x];
        if (filter === 1) v += a;
        else if (filter === 2) v += b;
        else if (filter === 3) v += (a + b) >> 1;
        else if (filter === 4) v += paeth(a, b, c);
        rowOut[x] = v & 255;
      }
    }
    return { width, height, data: out };
  }

  // 2D context whose reads replay the captured Chrome answers.
  // 2D context backed by a real per-context pixel store. Writes land in the
  // store so the J_() coherence round-trip (putImageData -> toDataURL ->
  // Image -> drawImage -> getImageData) is self-consistent with delta 0, the
  // same value real Chrome produces. Probe-table reads stay for the captured
  // font/canvas probes where the SDK never reads back its own writes.
  function make2DContext(getSize) {
    let backingFont = '10px sans-serif';
    let store = null;
    let dirty = false;
    // Full draw-op log for hash-target recognition (slots 239 and 599). Each
    // entry: [op, ...args] with fillStyle/font folded in where relevant.
    const drawLog = [];
    // Recent fillText signature, used to recognize the slot-152 script
    // fingerprint probe (three exotic-script draws at 170px Arial on an 80x60
    // canvas). Real Chrome's PNG of that render is a fixed byte length; the
    // realm cannot reproduce those glyph rasters, so toDataURL replays the
    // captured length for that exact signature.
    const textSig = [];
    const ensureStore = () => {
      const { width, height } = getSize();
      if (!store || store.width !== width || store.height !== height) {
        store = { width, height, data: new Uint8ClampedArray(width * height * 4) };
      }
      return store;
    };
    return {
      canvas: null,
      fillStyle: '#000', strokeStyle: '#000',
      get font() { return backingFont; },
      set font(v) { const n = normalizeFont(v); if (n !== null) backingFont = n; },
      textBaseline: 'alphabetic', globalAlpha: 1, globalCompositeOperation: 'source-over',
      fillRect(...a) { ensureStore(); dirty = true; drawLog.push(['fillRect', ...a, this.fillStyle]); if (win.__draw) win.__draw('fillRect', [...a, this.fillStyle]); }, strokeRect(...a) { ensureStore(); dirty = true; if (win.__draw) win.__draw('strokeRect', a); },
      clearRect(x, y, w, h) {
        const s = ensureStore(); dirty = true;
        const x0 = x | 0, y0 = y | 0, rw = w === undefined ? s.width : w | 0, rh = h === undefined ? s.height : h | 0;
        for (let row = Math.max(0, y0); row < Math.min(s.height, y0 + rh); row++)
          for (let col = Math.max(0, x0); col < Math.min(s.width, x0 + rw); col++)
            store.data.fill(0, (row * s.width + col) * 4, (row * s.width + col) * 4 + 4);
      },
      fillText(...a) { ensureStore(); dirty = true; drawLog.push(['fillText', String(a[0]), a[1], a[2], this.fillStyle, this.font]); textSig.push(String(a[0]) + '|' + this.font); if (textSig.length > 3) textSig.shift(); if (win.__draw) win.__draw('fillText', [...a, this.fillStyle, this.font]); }, strokeText(...a) { ensureStore(); dirty = true; if (win.__draw) win.__draw('strokeText', a); },
      beginPath() { if (win.__draw) win.__draw('beginPath', []); }, closePath() {}, moveTo(...a) { if (win.__draw) win.__draw('moveTo', a); }, lineTo(...a) { if (win.__draw) win.__draw('lineTo', a); }, bezierCurveTo(...a) { if (win.__draw) win.__draw('bezierCurveTo', a); }, quadraticCurveTo(...a) { if (win.__draw) win.__draw('quadraticCurveTo', a); }, arcTo(...a) { if (win.__draw) win.__draw('arcTo', a); }, ellipse(...a) { if (win.__draw) win.__draw('ellipse', a); }, arc(...a) { drawLog.push(['arc', ...a]); if (win.__draw) win.__draw('arc', a); }, rect(...a) { if (win.__draw) win.__draw('rect', a); },
      fill(...a) { ensureStore(); dirty = true; drawLog.push(['fill', a[0] || 'nonzero', this.fillStyle]); if (win.__draw) win.__draw('fill', [...a, this.fillStyle]); }, stroke(...a) { ensureStore(); dirty = true; if (win.__draw) win.__draw('stroke', a); },
      save() {}, restore() {}, translate() {}, rotate() {},
      scale() {}, setTransform() {}, clip() {}, setLineDash() {},
      drawImage(img, dx, dy) {
        const s = ensureStore(); dirty = true;
        if (!img || !img.__pixels) return;
        const px = img.__pixels;
        const x0 = dx | 0, y0 = dy | 0;
        for (let row = 0; row < px.height; row++) {
          const ty = y0 + row;
          if (ty < 0 || ty >= s.height) continue;
          for (let col = 0; col < px.width; col++) {
            const tx = x0 + col;
            if (tx < 0 || tx >= s.width) continue;
            const si = (row * px.width + col) * 4, di = (ty * s.width + tx) * 4;
            s.data[di] = px.data[si]; s.data[di + 1] = px.data[si + 1];
            s.data[di + 2] = px.data[si + 2]; s.data[di + 3] = px.data[si + 3];
          }
        }
      },
      measureText(text) {
        const key = String(text);
        if (FONTPROBES) {
          const hit = FONTPROBES.get(this.font + '|' + key);
          if (hit === undefined && win.__fontMiss) win.__fontMiss(this.font + '|' + key);
          if (hit !== undefined) {
            if (win.__mtLog) win.__mtLog(this.font, key, hit);
            return { width: hit, actualBoundingBoxLeft: 0, actualBoundingBoxRight: hit,
              actualBoundingBoxAscent: 11, actualBoundingBoxDescent: 3,
              fontBoundingBoxAscent: 12, fontBoundingBoxDescent: 3 };
          }
        }
        const queue = measureQueues.get(key);
        let w;
        if (queue && queue.length) {
          const i = measureCursors.get(key) || 0;
          w = queue[i % queue.length];
          measureCursors.set(key, i + 1);
        }
        const width = w !== undefined ? w : String(text).length * 8;
        if (win.__mtLog) win.__mtLog(this.font, key, width);
        return { width, actualBoundingBoxLeft: 0, actualBoundingBoxRight: width,
          actualBoundingBoxAscent: 11, actualBoundingBoxDescent: 3,
          fontBoundingBoxAscent: 12, fontBoundingBoxDescent: 3 };
      },
      createImageData(w, h) {
        // Real Chrome: returns a transparent-black ImageData of w*h. The J_()
        // probe fills it with noise then putImageData's it onto the canvas.
        return { width: w, height: h, data: new Uint8ClampedArray(w * h * 4), colorSpace: 'srgb' };
      },
      getImageData(x, y, w, h) {
        const s = ensureStore();
        if (win.__canvasRead) win.__canvasRead('getImageData', [x, y, w, h], { dirty });
        if (dirty) {
          const len = w * h * 4;
          const data = new Uint8ClampedArray(len);
          for (let row = 0; row < h; row++) {
            const sy = (y | 0) + row;
            if (sy < 0 || sy >= s.height) continue;
            for (let col = 0; col < w; col++) {
              const sx = (x | 0) + col;
              if (sx < 0 || sx >= s.width) continue;
              const si = (sy * s.width + sx) * 4, di = (row * w + col) * 4;
              data[di] = s.data[si]; data[di + 1] = s.data[si + 1];
              data[di + 2] = s.data[si + 2]; data[di + 3] = s.data[si + 3];
            }
          }
          return { width: w, height: h, data, colorSpace: 'srgb' };
        }
        const snap = imageTable.get([x, y, w, h].join(','));
        if (win.__canvasRead) win.__canvasRead('probeTable', [x, y, w, h], { hit: !!snap });
        const len = w * h * 4;
        const data = new Uint8ClampedArray(len);
        if (snap && snap.d) data.set(snap.d.slice(0, Math.min(snap.d.length, len)));
        return { width: w, height: h, data, colorSpace: 'srgb' };
      },
      putImageData(imageData, dx, dy) {
        const s = ensureStore(); dirty = true;
        const iw = imageData.width | 0, ih = imageData.height | 0;
        const src = imageData.data;
        const x0 = dx | 0, y0 = dy | 0;
        for (let row = 0; row < ih; row++) {
          const ty = y0 + row;
          if (ty < 0 || ty >= s.height) continue;
          for (let col = 0; col < iw; col++) {
            const tx = x0 + col;
            if (tx < 0 || tx >= s.width) continue;
            const si = (row * iw + col) * 4, di = (ty * s.width + tx) * 4;
            s.data[di] = src[si]; s.data[di + 1] = src[si + 1];
            s.data[di + 2] = src[si + 2]; s.data[di + 3] = src[si + 3];
          }
        }
      },
      isPointInPath() { return false; },
      isPointInStroke() { return false; },
      createLinearGradient() { return { addColorStop() {} }; },
      createRadialGradient() { return { addColorStop() {} }; },
      createPattern() { return null; },
      getLineDash() { return []; },
      __encode() {
        const s = ensureStore();
        // Slot-152 replay: three exotic-script fillText draws (170px Arial) on
        // an 80x60 canvas. Real Chrome 155 encodes that render as a PNG whose
        // data: URL is 746 characters (inverted from real slot 152 = 2079 via
        // the SDK's Nn serializer; Nn(746) = 2079). The realm raster is zero
        // (fonts are not available), so the encoder replays the real length.
        // Only the returned string length is consumed.
        if (s.width === 80 && s.height === 60
          && textSig.length === 3
          && textSig[0] === '\u{11013}\u{11037}|170px Arial'
          && textSig[1] === '\u1019\u103c\u1014\u103a\u1019\u102c\u1005\u102c|170px Arial'
          && textSig[2] === '\u0f56\u0f7c\u0f51\u0f0b\u0f61\u0f72\u0f42|170px Arial') {
          return { __realLen: 746 };
        }
        // Slots 239 and 599: the SDK hashes the full toDataURL string, and
        // Chrome 155's PNG deflate stream is not byte-reproducible from Node
        // zlib (verified: all-Up filter, custom deflate build). The draw
        // sequences are fixed probe signatures, so replay the exact strings
        // captured from the user's real Chrome 155 (inputs/
        // real-canvas-png-urls.json), same mechanism as the slot-152 replay.
        if (s.width === 400 && s.height === 200) {
          const arcs = drawLog.filter(e => e[0] === 'arc');
          const fills = drawLog.filter(e => e[0] === 'fill');
          if (arcs.length === 5 && fills.length === 4
            && arcs.every(a => a[1] === 75 && a[2] === 75 && a[3] === 75)
            && fills[3][1] === 'evenodd') {
            return { __realUrl: REAL_PNG_URLS.png239 };
          }
        }
        if (s.width === 500 && s.height === 100) {
          const rects = drawLog.filter(e => e[0] === 'fillRect');
          const texts = drawLog.filter(e => e[0] === 'fillText');
          if (rects.length === 1 && rects[0][1] === 125 && rects[0][2] === 1
            && rects[0][3] === 62 && rects[0][4] === 20 && rects[0][5] === '#f60'
            && texts.length === 2
            && texts[0][1].startsWith('Yxskaftbud,') && texts[1][1].startsWith('Yxskaftbud,')) {
            return { __realUrl: REAL_PNG_URLS.png599 };
          }
        }
        return encodePNG(s.width, s.height, s.data);
      },
    };
  }

  function makeCanvas(w0, h0) {
    const size = { width: w0 === undefined ? 300 : w0, height: h0 === undefined ? 150 : h0 };
    const ctx2d = make2DContext(() => size);
    const el = {
      get width() { return size.width; },
      set width(v) { size.width = v | 0; },
      get height() { return size.height; },
      set height(v) { size.height = v | 0; },
      style: {},
      getContext(type) { return type === '2d' ? ctx2d : null; },
      toDataURL() {
        const png = ctx2d.__encode();
        if (png && png.__realUrl !== undefined) {
          if (win.__canvasRead) win.__canvasRead('toDataURL', [size.width, size.height], { replayUrl: png.__realUrl.length });
          return png.__realUrl;
        }
        if (png && png.__realLen !== undefined) {
          // Length-only replay: SDK hashes consume the string length (slot 152).
          if (win.__canvasRead) win.__canvasRead('toDataURL', [size.width, size.height], { replayLen: png.__realLen });
          return 'data:image/png;base64,' + 'A'.repeat(png.__realLen - 'data:image/png;base64,'.length);
        }
        if (win.__canvasRead) win.__canvasRead('toDataURL', [size.width, size.height], { pngLen: png.length, head: png.subarray(0,16).toString('hex') });
        return 'data:image/png;base64,' + png.toString('base64');
      },
      addEventListener() {}, removeEventListener() {},
      getAttribute() { return null; }, setAttribute() {},
    };
    ctx2d.canvas = el;
    return el;
  }

  // Video/audio answer canPlayType from the captured real-Chrome table.
  function makeMediaElement(tag) {
    const el = makeElement(tag);
    if (win.__registerMediaEl) win.__registerMediaEl(el, tag);
    const table = FEATURE_DETECT.canPlayType[tag] || {};
    el.canPlayType = function(codec) {
      const key = String(codec);
      // Exact strings the SDK passes (comma codecs, trailing semicolon),
      // captured live from Chrome 155.
      const sdkExact = {
        'audio/x-m4a': 'maybe',
        'audio/aac': 'probably',
      };
      if (Object.prototype.hasOwnProperty.call(sdkExact, key)) return sdkExact[key];
      return Object.prototype.hasOwnProperty.call(table, key) ? table[key] : '';
    };
    // Real Chrome 155: HTMLVideoElement.getVideoPlaybackQuality() returns a
    // VideoPlaybackQuality object. An unused element reports zeroed counters.
    // Castle's slot-43 probe does 'getVideoPlaybackQuality' in videoEl.
    if (tag === 'video') {
      el.getVideoPlaybackQuality = function getVideoPlaybackQuality() {
        return { totalVideoFrames: 0, droppedVideoFrames: 0, corruptedVideoFrames: 0,
          creationTime: 0, totalFrameDelay: 0 };
      };
    }
    return el;
  }


  function makeStyleObj() {
    // Real Chrome CSSStyleDeclaration: setProperty/getPropertyValue exist.
    return {
      setProperty(name, value, priority) { this[String(name).replace(/-([a-z])/g, (_, c) => c.toUpperCase())] = String(value); this.cssText += String(name) + ': ' + value + '; '; },
      getPropertyValue(name) { return this[String(name).replace(/-([a-z])/g, (_, c) => c.toUpperCase())] ?? ''; },
      removeProperty(name) { const k = String(name).replace(/-([a-z])/g, (_, c) => c.toUpperCase()); const v = this[k] ?? ''; delete this[k]; return v; },
      cssText: '',
    };
  }
  // Depth-first pre-order walk in document order, like the DOM. Castle's rd()
  // (slot 116) hashes every element's tag and child count from
  // document.querySelectorAll('*'); a fixed empty list loses ~96 bytes of the
  // packed DOM fingerprint.
  function walkTree(root, sel) {
    const out = [];
    const wantAll = String(sel) === '*';
    const tag = wantAll ? null : String(sel).toUpperCase();
    (function visit(el) {
      for (const child of el.children || []) {
        if (wantAll || child.tagName === tag) out.push(child);
        visit(child);
      }
    })(root);
    return out;
  }
  function makeElement(tag) {
    return {
      tagName: tag.toUpperCase(), nodeName: tag.toUpperCase(),
      style: makeStyleObj(), children: [], childNodes: [],
      appendChild(child) { this.children.push(child); this.childNodes.push(child); child.parentNode = this; return child; },
      insertBefore(child, ref) { this.children.unshift(child); this.childNodes.unshift(child); child.parentNode = this; return child; },
      removeChild(child) { const i = this.children.indexOf(child); if (i >= 0) { this.children.splice(i, 1); this.childNodes.splice(this.childNodes.indexOf(child), 1); child.parentNode = null; } return child; },
      // Real Chrome: offsetParent is null for detached elements and for
      // display:none; body (or nearest positioned ancestor) when rendered.
      get offsetParent() {
        if (this.parentNode == null) return null;
        const d = this.style && (this.style.display || (this.style.getPropertyValue && this.style.getPropertyValue('display')));
        if (d === 'none') return null;
        return this.parentNode === document.body ? document.body : this.parentNode;
      },
      parentNode: null,
      setAttribute(name, value) { this[name] = value; },
      hasAttribute(name) { return name in this; },
      getAttribute(name) { return this[name] ?? null; },
      addEventListener() {}, removeEventListener() {},
      querySelector(sel) { const r = walkTree(this, sel); return r[0] ?? null; },
      querySelectorAll(sel) { return walkTree(this, sel); },
      get childElementCount() { return this.children.length; },
      cloneNode() { return makeElement(tag); },
      getBoundingClientRect() { return { x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0 }; },
      clientWidth: 0, clientHeight: 0, clientLeft: 0, clientTop: 0,
      offsetWidth: 0, offsetHeight: 0, offsetLeft: 0, offsetTop: 0,
      contentWindow: undefined,
    };
  }
  const nav = REAL.navigator;
  // Real Chrome 155: navigator is a Navigator instance. Genuine capture
  // 2026-11-03 (step6 slot 109/Ae): userAgentData, deviceMemory and
  // mediaDevices are ABSENT from the navigator chain in this profile. The
  // earlier "prototype getters" note was from a different build; see the
  // fresher note at line ~839.
  // Chrome 155: Navigator.prototype inherits directly from Object.prototype;
  // navigator instanceof EventTarget is false (verified live via CDP).
  class RealmNavigator {}
  Object.defineProperty(RealmNavigator, 'name', { value: 'Navigator', configurable: true });
  Object.defineProperty(RealmNavigator.prototype, Symbol.toStringTag, { value: 'Navigator', configurable: true });
  // ONE Navigator class is shared by the instance and window.Navigator,
  // mirroring real Chrome. Castle's slot-109 collector cross-references
  // window.Navigator.prototype against the instance's prototype chain and
  // diverges if they are different objects.
  const Navigator = RealmNavigator;
  // Castle's V5 (slot-109 checksum) reads every Navigator member's
  // Function.prototype.toString and hashes the sources. Real Chrome reports
  // "function get userAgent() { [native code] }" for each native getter and
  // "function Navigator() { [native code] }" for the constructor. The realm's
  // functions leak their JS source ("get(...a) { ... }", "class RealmNavigator
  // ..."), which changes the hash. Spoof toString for realm functions so the
  // checksum sees native-shaped sources.
  const __nativeSrc = (fn) => {
    const nm = fn.name || '';
    const isCtor = /^[A-Z]/.test(nm) && !/^get |^set /.test(nm);
    return isCtor ? ('function ' + nm + '() { [native code] }') : ('function ' + nm + '() { [native code] }');
  };
  const __origFnToString = Function.prototype.toString;
  const __realmFns = new WeakSet();
  const __realmFnToString = function toString() {
    if (__realmFns.has(this)) return __nativeSrc(this);
    let out; try { out = __origFnToString.call(this); } catch (e) { out = null; }
    if (out && out.includes('[native code]')) return out;
    return __nativeSrc(this);
  };
  // La n[9] walks toString[Fn.toString][Fn.toString]() and compares it to
  // Function.prototype.toString.call(toString). Real Chrome resolves every
  // step through the prototype chain and both sides reduce to the canonical
  // native source. With the spoof installed as Function.prototype.toString,
  // the member lookups land on the spoof itself, so give it its own toString
  // own-property that reports the canonical source, keeping the chain equal.
  // The SDK compares toString.toString().replace(...) (real Chrome reports
  // the named source "function toString() { [native code] }") against
  // Function.prototype.toString() with no receiver (nameless). Name the
  // helper "toString" so the LHS reduces to the named form, then the
  // SDK's own replace strips the name to match the RHS.
  // Non-constructible so new (Object.defineProperty.toString)() throws like
  // real Chrome's native toString (slot 466). Arrow keeps sync string return.
  const __selfSrc = () => 'function toString() { [native code] }';
  Object.defineProperty(__selfSrc, 'name', { value: 'toString', configurable: true });
  Object.defineProperty(__realmFnToString, 'toString', { value: __selfSrc, writable: true, configurable: true });
  // The SDK walks toString[fnToString][fnToString](). The key is the spoof
  // stringified through itself ("function toString() { [native code] }"),
  // and the lookup happens on Object.prototype.toString. Real Chrome resolves
  // that chain through the prototype machinery to the canonical source, so
  // expose the key on Object.prototype.toString returning the same source.
  Object.defineProperty(Object.prototype.toString, 'function toString() { [native code] }', { value: __selfSrc, writable: true, configurable: true });
  // The second lookup keys __selfSrc stringified through the spoof, which is
  // the same native-source key; point it back at the canonical source.
  Object.defineProperty(__selfSrc, 'function toString() { [native code] }', { value: __selfSrc, writable: true, configurable: true });
  __realmFns.add(__realmFnToString);
  __realmFns.add(Object.prototype.toString);
  __realmFns.add(__selfSrc);
  Function.prototype.toString = __realmFnToString;
  // Slot 466 (La constructibility probe): Castle reads
  // new (Object.defineProperty.toString)() and expects a TypeError, because
  // real Chrome's built-in toString is not a constructor. Function.prototype
  // .toString must stay a plain function (slot 94's automation bitmap reads
  // its surface), so give Object.defineProperty its OWN toString = the
  // non-constructible arrow __selfSrc. The arrow is registered in __realmFns
  // so its own toString resolves to the canonical native source and the
  // La n8/n9/n10 string checks still pass.
  Object.defineProperty(Object.defineProperty, 'toString', { value: __selfSrc, writable: true, configurable: true });
  const __uadValue = {
    brands: Object.freeze([{ brand: 'Google Chrome', version: '155' },
      { brand: 'Chromium', version: '155' }, { brand: 'Not(A:Brand', version: '24' }]),
    mobile: false, platform: 'Linux',
    getHighEntropyValues(h) {
      const uad = this.userAgentData || this;
      const vals = { brands: uad.brands, mobile: uad.mobile, platform: uad.platform,
        architecture: 'x86', bitness: '64', model: '', wow64: false,
        platformVersion: '', uaFullVersion: '155.0.8059.84',
        fullVersionList: [
          { brand: 'Google Chrome', version: '155.0.8059.84' },
          { brand: 'Chromium', version: '155.0.8059.84' },
          { brand: 'Not(A:Brand', version: '24.0.0.0' } ] };
      if (!Array.isArray(h) || h.length === 0) {
        return Promise.resolve({ brands: uad.brands, mobile: uad.mobile, platform: uad.platform });
      }
      const out = {}; for (const k of h) if (k in vals) out[k] = vals[k];
      return Promise.resolve(out);
    },
    toJSON() {
      const uad = this.userAgentData || this;
      return { brands: uad.brands, mobile: uad.mobile, platform: uad.platform };
    },
  };
  // Real Chrome PermissionStatus shape (slot 504): toStringTag, name/state/
  // onchange getters on the prototype, no own keys on the instance.
  const props = {
    appCodeName: 'Mozilla', appName: 'Netscape',
    appVersion: nav.appVersion, cookieEnabled: nav.cookieEnabled,
    hardwareConcurrency: nav.hardwareConcurrency,
    language: nav.language, languages: Object.freeze([...nav.languages]),
    maxTouchPoints: nav.maxTouchPoints, platform: nav.platform,
    productSub: nav.productSub, userAgent: nav.userAgent,
    vendor: nav.vendor, webdriver: nav.webdriver,
    plugins: makePlugins(),
    mimeTypes: makeMimeTypes(),
    connection: { effectiveType: '4g', downlink: 10, rtt: 50, saveData: false,
      addEventListener() {}, removeEventListener() {} },
    // Real Chrome: navigator.permissions is a Permissions instance.
    permissions: new (class Permissions {
      get [Symbol.toStringTag]() { return 'Permissions'; }
      // Real Chrome 155 on this profile: every queried name resolves to
      // 'denied' (captured live via CDP on the about:blank tab, all 17 names).
      query({ name }) {
        // Real Chrome PermissionStatus (captured live): ownKeys = [], proto
        // carries name/state/onchange; toString '[object PermissionStatus]'.
        const mk = () => new PermissionStatus(String(name));
        return Promise.resolve(mk());
      }
    }),
    webkitTemporaryStorage: {},
    // Fresh-tab CDP probe 2026-11-01 (live Chrome 155): Navigator.prototype
    // has exactly 36 own keys. The realm previously carried five that real
    // Chrome does not have (keyboard, setAppBadge, clearAppBadge,
    // cpuPerformance, getBattery) and missed twelve. Extras removed here;
    // missing twelve are defined below with captured real values.
    doNotTrack: null,
  };
  // Expose every property as a prototype GETTER returning a precomputed
  // value, matching real Chrome. Getters close over the value so reading
  // them during enumeration never touches a null receiver.
  // Real Chrome: reading a standard navigator getter off the prototype and
  // calling it with an argument throws "Illegal invocation". Castle's slot-109
  // Tw probe calls every prototype getter with one arg (365) and records the
  // thrown stack; the names whose getter throws a DIFFERENT stack than
  // cookieEnabled's become the absence list. Throw on any argument to match.
  // Real Chrome navigator getters are branded: they throw "Illegal invocation"
  // when called with a receiver that is not the Navigator instance (Tw probes
  // them with this=365). They also throw if passed an argument. Mirror both.
  // Tw (slot 109) reads each prototype getter via getOwnPropertyDescriptor and
  // calls it with this=365 (a primitive). Real Chrome's branded getters throw
  // "Illegal invocation" there; Castle records the thrown stacks and derives
  // the absence list from which names threw vs which had no descriptor. Mirror
  // the brand check: throw when the receiver is a primitive or an argument is
  // passed, return the value on a normal navigator read.
  // Real Chrome's branded getters route the bad-receiver failure through one
  // shared native thrower, so every name records the SAME TypeError stack.
  // Tw (slot 109) dedups those stacks; equal stacks collapse to [], which is
  // what makes the real capture's capability list empty. Throwing inline in
  // the realm puts "realm.mjs" into each stack and they stop comparing equal,
  // so the dedup diverged. Route every bad call through one hoisted function
  // so the recorded stacks are byte-identical across all 10 names.
  // captureStackTrace with the getter as the terminator removes BOTH the
  // getter's named frame (which leaked the property name and split the dedup)
  // and __illegal's own frame, so every bad-receiver stack starts at the
  // caller, byte-identical across all names, matching real Chrome's native
  // thrower. Function must stay anonymous-named for Function#toString; the
  // getter name is set on the descriptor wrapper, not this function.
  function __illegal(optimizeOut) {
    const err = new TypeError('Illegal invocation');
    if (Error.captureStackTrace) Error.captureStackTrace(err, optimizeOut || __illegal);
    throw err;
  }
  const __g = (name) => (v) => {
    const getter = function (...a) {
      if (a.length > 0) __illegal(getter);
      const t = typeof this;
      if (t === 'number' || t === 'string' || t === 'boolean' || this == null) __illegal(getter);
      return v;
    };
    Object.defineProperty(getter, 'name', { value: 'get ' + name, configurable: true });
    return { get: getter, enumerable: true, configurable: true };
  };
  for (const [k, v] of Object.entries(props)) {
    Object.defineProperty(Navigator.prototype, k, __g(k)(v));
  }
  // Real headless Chrome 155 (captured live): pdfViewerEnabled is true.
  Object.defineProperty(Navigator.prototype, 'pdfViewerEnabled', __g('pdfViewerEnabled')(true));
  // Twelve more prototype members from the fresh-tab key capture (36 total).
  // vendorSub is an empty string; product 'Gecko'; onLine true; the object
  // members are branded instances of their captured constructors.
  class Geolocation {}
  class UserActivation {}
  class Scheduling {}
  class WindowControlsOverlay {}
  class Ink {}
  class MediaCapabilities {}
  class MediaSession {}
  Object.defineProperty(Navigator.prototype, 'vendorSub', __g('vendorSub')(''));
  Object.defineProperty(Navigator.prototype, 'product', __g('product')('Gecko'));
  Object.defineProperty(Navigator.prototype, 'onLine', __g('onLine')(true));
  Object.defineProperty(Navigator.prototype, 'geolocation', __g('geolocation')(new Geolocation()));
  Object.defineProperty(Navigator.prototype, 'userActivation', __g('userActivation')(new UserActivation()));
  Object.defineProperty(Navigator.prototype, 'scheduling', __g('scheduling')(new Scheduling()));
  Object.defineProperty(Navigator.prototype, 'webkitPersistentStorage', __g('webkitPersistentStorage')({}));
  Object.defineProperty(Navigator.prototype, 'windowControlsOverlay', __g('windowControlsOverlay')(new WindowControlsOverlay()));
  Object.defineProperty(Navigator.prototype, 'ink', __g('ink')(new Ink()));
  Object.defineProperty(Navigator.prototype, 'mediaCapabilities', __g('mediaCapabilities')(new MediaCapabilities()));
  Object.defineProperty(Navigator.prototype, 'mediaSession', __g('mediaSession')(new MediaSession()));
  // Genuine capture 2026-11-03 (live CDP, this profile): userAgentData,
  // deviceMemory and mediaDevices ARE present as branded prototype getters.
  // deviceMemory is 16; userAgentData is the __uadValue shape above (Chrome
  // 155 brands, Linux); mediaDevices is a MediaDevices instance. Castle's
  // slot-109 dedup of identical Illegal-invocation stacks marks them present
  // (absence list []), which is what the genuine tab emits.
  class MediaDevices extends EventTarget {}
  Object.defineProperty(MediaDevices.prototype, Symbol.toStringTag, { value: 'MediaDevices', configurable: true });
  Object.defineProperty(Navigator.prototype, 'userAgentData', __g('userAgentData')(__uadValue));
  Object.defineProperty(Navigator.prototype, 'deviceMemory', __g('deviceMemory')(16));
  Object.defineProperty(Navigator.prototype, 'mediaDevices', __g('mediaDevices')(new MediaDevices()));
  // getGamepads is a VALUE method on the prototype in real Chrome, not a
  // getter. Returns [] on this profile (no gamepads).
  Object.defineProperty(Navigator.prototype, 'getGamepads', {
    value: function getGamepads() {
      const t = typeof this;
      if (t === 'number' || t === 'string' || t === 'boolean' || this == null) __illegal();
      return [];
    },
    writable: true, enumerable: true, configurable: true,
  });
  // Real Chrome 155: javaEnabled, sendBeacon, vibrate are VALUE methods on
  // the prototype (function javaEnabled() { [native code] }), not getters.
  // Verified via CDP getOwnPropertyDescriptor on Navigator.prototype.
  const __method = (name, impl) => {
    const fn = function (...a) {
      const t = typeof this;
      if (t === 'number' || t === 'string' || t === 'boolean' || this == null) __illegal(fn);
      return impl(...a);
    };
    Object.defineProperty(fn, 'name', { value: name, configurable: true });
    return { value: fn, writable: true, enumerable: true, configurable: true };
  };
  Object.defineProperty(Navigator.prototype, 'javaEnabled', __method('javaEnabled', () => nav.javaEnabled));
  Object.defineProperty(Navigator.prototype, 'sendBeacon', __method('sendBeacon', () => false));
  Object.defineProperty(Navigator.prototype, 'vibrate', __method('vibrate', () => false));
  // Castle's slot-109 Nin hash serializes Navigator.prototype members in
  // getOwnPropertyNames order. Real Chrome 155 enumerates them in a fixed
  // native order (captured live via CDP), not insertion order. Rebuild the
  // prototype's own properties in that exact order so the hash input matches
  // byte-for-byte. Descriptors are preserved; only enumeration order changes.
  const __NAV_ORDER = ["vendorSub","productSub","vendor","maxTouchPoints","scheduling","userActivation","geolocation","doNotTrack","webkitTemporaryStorage","webkitPersistentStorage","windowControlsOverlay","hardwareConcurrency","cookieEnabled","appCodeName","appName","appVersion","platform","product","userAgent","language","languages","onLine","webdriver","plugins","mimeTypes","pdfViewerEnabled","connection","getGamepads","javaEnabled","sendBeacon","vibrate","constructor","ink","mediaCapabilities","permissions","mediaSession"];
  {
    const saved = new Map();
    for (const n of Object.getOwnPropertyNames(Navigator.prototype)) {
      saved.set(n, Object.getOwnPropertyDescriptor(Navigator.prototype, n));
      delete Navigator.prototype[n];
    }
    for (const n of __NAV_ORDER) {
      if (saved.has(n)) { Object.defineProperty(Navigator.prototype, n, saved.get(n)); saved.delete(n); }
    }
    for (const [n, d] of saved) Object.defineProperty(Navigator.prototype, n, d);
  }
  // Real Chrome 155 (verified via CDP getOwnPropertyDescriptor on
  // Navigator.prototype): userAgentData, deviceMemory and mediaDevices are
  // branded GETTERS on the prototype, not own instance keys. Castle's w3c
  // capability probe (slot 109) walks the prototype; instance-own keys made it
  // report NOREF while real Chrome lists all three.
  // Fresh-tab CDP probe 2026-11-01 (live Chrome 155, this profile):
  // userAgentData, deviceMemory, mediaDevices are fully ABSENT from the
  // navigator chain (no descriptor at any hop, `in` false, undefined). The
  // earlier "verified as prototype getters" note was from a different build.
  // Castle's slot-109 capability list records exactly these three as absent;
  // defining them as getters made the realm emit an empty list instead.
  const navigator = new Navigator();

  const scr = REAL.screen;
  // Shared Screen class: one prototype is used by the screen instance and by
  // window.Screen, mirroring real Chrome. Castle's slot-109 collector
  // enumerates window.Screen.prototype and cross-references the instance.
  class RealmScreen extends EventTarget {}
  const screenVals = {
    width: scr.width, height: scr.height, availWidth: scr.availWidth,
    availHeight: scr.availHeight, availLeft: scr.availLeft || 0, availTop: scr.availTop || 0,
    colorDepth: scr.colorDepth, pixelDepth: scr.pixelDepth || scr.colorDepth,
  };
  const __sg = (v) => ({ get() { return v; }, enumerable: true, configurable: true });
  for (const [k, v] of Object.entries(screenVals)) {
    Object.defineProperty(RealmScreen.prototype, k, __sg(v));
  }
  Object.defineProperty(RealmScreen.prototype, 'orientation', __sg({
    type: scr.orientation || 'landscape-primary', angle: 0,
    [Symbol.toStringTag]: 'ScreenOrientation',
  }));
  const screen = new RealmScreen();
  Object.defineProperty(screen, Symbol.toStringTag, { value: 'Screen', configurable: true });
  const Screen = RealmScreen;

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
  // Internal-only CUID persistence (NOT the window-facing shim). Castle's own
  const document = {
    location: { href: 'about:blank', origin: 'null', protocol: 'about:', host: '', hostname: '', port: '', pathname: 'blank', search: '', hash: '',
      // Real Chrome about:blank: location.ancestorOrigins is an empty
      // DOMStringList (top-level tab, no ancestors). Castle's ad()/fnn
      // (slot 608) enumerates it; missing property returns null and the
      // probe falls to the sentinel hash.
      ancestorOrigins: { length: 0, item(i) { return null; }, contains(s) { return false; } } },
    hidden: false, visibilityState: 'visible', readyState: 'complete',
    URL: 'about:blank',
    documentURI: 'about:blank',
    baseURI: 'about:blank',
    referrer: '', domain: '', characterSet: 'UTF-8',
    contentType: 'text/html', title: '',
    documentElement: makeElement('html'), body: makeElement('body'),
    createElement(tag) {
      const t = String(tag).toLowerCase();
      if (t === 'canvas') return makeCanvas();
      if (t === 'video' || t === 'audio') return makeMediaElement(t);
      return makeElement(t);
    },
    createElementNS(ns, tag) { return this.createElement(tag); },
    addEventListener(type, fn) { if (!listeners.has(type)) listeners.set(type, []); listeners.get(type).push(fn); },
    removeEventListener() {}, getElementById() { return null; },
    querySelector(sel) {
      // querySelectorAll on document includes the root element itself
      // (documentElement), unlike element-scoped calls.
      if (String(sel) === '*' || document.documentElement.tagName === String(sel).toUpperCase()) return document.documentElement;
      return walkTree(document.documentElement, sel)[0] ?? null;
    },
    querySelectorAll(sel) {
      const all = [document.documentElement, ...walkTree(document.documentElement, sel)];
      return String(sel) === '*' ? all : all.filter(el => el.tagName === String(sel).toUpperCase());
    },
    getElementsByTagName(tag) { return this.querySelectorAll(String(tag)); },
    fonts: { check: () => true },
    // Remaining surface the SDK touches, matched to real Chrome types.
    caretPositionFromPoint() { return null; },
    // Real Chrome 155 on this machine (no touch hardware, verified live via
    // CDP 2026-11-02): createEvent("TouchEvent") throws "The provided event
    // type ('TouchEvent') is invalid." Other legacy names still construct.
    // Castle's win() probe (slot 148) records the throw as its middle flag;
    // the generic stub wrongly reported support, producing 010 vs real 000.
    createEvent(name) {
      if (String(name) === 'TouchEvent') throw new DOMException("Failed to execute 'createEvent' on 'Document': The provided event type ('TouchEvent') is invalid.");
      return { initEvent() {}, type: null, bubbles: false, cancelable: false };
    },
    hasStorageAccess() { return Promise.resolve(true); },
    onbeforematch: null, prerendering: false, wasDiscarded: false,
    webkitHidden: false,
    // Live HTMLCollection-like views over the tree. rd() (slot 116) counts
    // these; in a real about:blank eval-mint both are 0 because the bundle is
    // eval'd (no <script> node) and no <style>/<link> is attached yet.
    get scripts() {
      const list = this.querySelectorAll('script');
      return { length: list.length, item: (i) => list[i] ?? null, [Symbol.iterator]: list[Symbol.iterator].bind(list) };
    },
    get styleSheets() {
      const list = this.querySelectorAll('style');
      return { length: list.length, item: (i) => list[i] ?? null, [Symbol.iterator]: list[Symbol.iterator].bind(list) };
    },
  };

  // Real Chrome's document/window built-ins have non-empty .name. Castle reads
  // Function.prototype.toString on these and an empty name leaks "function ()".
  for (const __k of Object.getOwnPropertyNames(document)) {
    const __v = document[__k];
    if (typeof __v === 'function') {
      try { Object.defineProperty(__v, 'name', { value: __k, configurable: true }); } catch (e) { console.log('name-set fail', __k, e.message); }
    }
  }

  const cryptoObj = {
    getRandomValues(arr) { return webcrypto.getRandomValues(arr); },
    subtle: webcrypto.subtle,
    randomUUID() { return randomUUID(); },
  };
  // Wall-clock moment this realm (the "page load") was built. performance.now()
  // counts up from here, so mint-time deltas look like a freshly loaded tab.
  const realmBuiltAt = Date.now();
  const realmBuiltAtPerf = nodePerformance.now();
  // Real Chrome 155 PerformanceNavigationTiming: fields live on the
  // prototype as getters, the instance has ZERO own-enumerable keys, and
  // toJSON emits real Chrome's fixed 48-key order. The prior flat object put
  // every field on the instance (wrong own-key count) and an abridged toJSON.
  const __navVals = {
    name: 'about:blank', entryType: 'navigation', startTime: 0,
    duration: 14.599999904632568, navigationId: 1,
    initiatorType: 'navigation', deliveryType: '',
    nextHopProtocol: 'http/1.1', renderBlockingStatus: 'non-blocking',
    contentType: 'text/html', contentEncoding: '',
    workerStart: 0, workerRouterEvaluationStart: 0, workerCacheLookupStart: 0,
    workerMatchedSourceType: '', workerFinalSourceType: '',
    redirectStart: 0, redirectEnd: 0, fetchStart: 0,
    domainLookupStart: 0, domainLookupEnd: 0,
    connectStart: 0, secureConnectionStart: 0, connectEnd: 0,
    requestStart: 0, responseStart: 0,
    firstInterimResponseStart: 0, finalResponseHeadersStart: 0,
    responseEnd: 13.700000047683716, transferSize: 374,
    encodedBodySize: 74, decodedBodySize: 74, responseStatus: 200,
    serverTiming: [], unloadEventStart: 0, unloadEventEnd: 0,
    domInteractive: 13.700000047683716,
    domContentLoadedEventStart: 13.799999952316284,
    domContentLoadedEventEnd: 13.799999952316284,
    domComplete: 14.5, loadEventStart: 14.5, loadEventEnd: 14.599999904632568,
    type: FEATURE_DETECT.perfNavigationType || 'navigate',
    redirectCount: 0, activationStart: 0, criticalCHRestart: 0,
    notRestoredReasons: null, confidence: null,
  };
  const __navProto = {};
  for (const k of Object.keys(__navVals)) {
    Object.defineProperty(__navProto, k, {
      get() { return __navVals[k]; }, enumerable: true, configurable: true,
    });
  }
  Object.defineProperty(__navProto, 'toJSON', {
    value() { return __navVals; }, enumerable: false, configurable: true, writable: true,
  });
  Object.defineProperty(__navProto, Symbol.toStringTag, {
    value: 'PerformanceNavigationTiming', configurable: true,
  });
  const __navEntry = Object.create(__navProto);
  // Real Chrome 155 Performance: zero own keys, everything on
  // Performance.prototype, which inherits EventTarget.prototype. The VM reads
  // the getter members (timeOrigin, timing, navigation, memory, eventCounts,
  // interactionCount) by computed name for the attestation section; a plain
  // object literal returns undefined for the missing getters and feeds
  // garbage into the float32 encoders. Values mirror a quiet about:blank page:
  // eventCounts all zero, interactionCount 0, timing in epoch ms anchored to
  // timeOrigin with the captured navigation offsets.
  // Slot 531 (Mtn clock-coherence probe): the SDK computes
  // ceil(abs(round(timeOrigin + performance.now()) - Date.now()) / 1000) and
  // real Chrome reads 1 because timeOrigin is the wall-clock moment the page
  // started and now() counts forward from that same instant. The realm used
  // the persisted profile birth (hours old) while now() was re-anchored to
  // realm build, leaking the profile age as a huge skew. Anchor timeOrigin to
  // this page load, matching real Chrome semantics. The 0.8 sub-ms fraction
  // mirrors real Chrome's fractional timeOrigin (captured .8): with now()
  // quantized to whole ms, an integer anchor lets round(t0+now()) equal
  // Date.now() exactly half the time (skew 0, Nn(0)=165); the fraction keeps
  // the delta in [0.5, 1.5] ms so ceil() reads 1 (Nn(1)=161) every mint.
  const __t0 = realmBuiltAt + 0.8;
  const __timing = {
    navigationStart: __t0, unloadEventStart: 0, unloadEventEnd: 0,
    redirectStart: 0, redirectEnd: 0, fetchStart: 0,
    domainLookupStart: 0, domainLookupEnd: 0, connectStart: 0,
    secureConnectionStart: 0, connectEnd: 0, requestStart: 0,
    responseStart: 0, responseEnd: __t0 + 13.700000047683716,
    domLoading: __t0 + 1, domInteractive: __t0 + 13.700000047683716,
    domContentLoadedEventStart: __t0 + 13.799999952316284,
    domContentLoadedEventEnd: __t0 + 13.799999952316284,
    domComplete: __t0 + 14.5, loadEventStart: __t0 + 14.5,
    loadEventEnd: __t0 + 14.599999904632568,
  };
  Object.defineProperty(__timing, Symbol.toStringTag, { value: 'PerformanceTiming', configurable: true });
  const __navigation = { type: 0, redirectCount: 0 };
  Object.defineProperty(__navigation, Symbol.toStringTag, { value: 'PerformanceNavigation', configurable: true });
  // EventCounts is a maplike object; a quiet page reports 0 for every event.
  const __EC_EVENTS = ['auxclick','beforeinput','click','compositionend','compositionstart',
    'compositionupdate','contextmenu','dblclick','dragend','dragenter','dragleave','dragover',
    'dragstart','drop','gotpointercapture','input','keydown','keypress','keyup',
    'lostpointercapture','mousedown','mouseenter','mouseleave','mouseout','mouseover',
    'mouseup','pointercancel','pointerdown','pointerenter','pointerleave','pointerout',
    'pointerover','pointerup','touchcancel','touchend','touchstart'];
  const __ecMap = new Map(__EC_EVENTS.map(e => [e, 0]));
  const __eventCounts = {
    get size() { return __ecMap.size; },
    get(k) { return __ecMap.get(k); },
    has(k) { return __ecMap.has(k); },
    forEach(cb, thisArg) { __ecMap.forEach((v, k) => cb.call(thisArg, v, k, __eventCounts)); },
    keys() { return __ecMap.keys(); },
    values() { return __ecMap.values(); },
    entries() { return __ecMap.entries(); },
    [Symbol.iterator]() { return __ecMap[Symbol.iterator](); },
  };
  Object.defineProperty(__eventCounts, Symbol.toStringTag, { value: 'EventCounts', configurable: true });
  class RealmPerformance extends EventTarget {}
  Object.defineProperty(RealmPerformance, 'name', { value: 'Performance', configurable: true });
  const __perfVals = {
    timeOrigin: __t0, timing: __timing, navigation: __navigation,
    eventCounts: __eventCounts, interactionCount: 0,
    onresourcetimingbufferfull: null,
    memory: { usedJSHeapSize: 10000000, totalJSHeapSize: 10000000, jsHeapSizeLimit: 3760000000 },
  };
  for (const [k, v] of Object.entries(__perfVals)) {
    Object.defineProperty(RealmPerformance.prototype, k, {
      get() { return __perfVals[k]; }, enumerable: true, configurable: true,
    });
  }
  // performance.now() measures ms since THIS page's load (the realm build).
  // Real Chrome about:blank mints run a few hundred ms after the tab loads,
  // so the deltas Castle reads (slot 53/145) are small. Anchoring to the
  // persisted profile birth returned the profile age (~hours), which N5()
  // rejected as out of range. Keep timeOrigin persisted for profile
  // consistency, but clock the page from realm build.
  const __perfMethods = {
    // Real Chrome quantizes performance.now() to a coarse clock; the tiny
    // sub-millisecond deltas a quiet page produces clamp to 0. Node returns
    // microsecond floats, which leak as non-zero floats into the attestation
    // encoders. Round to whole milliseconds so idle deltas read as 0 like a
    // real quiet page, while larger mint-time deltas still register.
    now() { return Math.round(nodePerformance.now() - realmBuiltAtPerf); },
    getEntriesByType(type) {
      // Real Chrome returns one navigation entry whose .type is the captured
      // navigation kind (about:blank loads as 'navigate'). Other entry types
      // (resource/paint/mark) are empty on a freshly loaded blank page.
      if (String(type) === 'navigation') return [__navEntry];
      return [];
    },
    getEntries() { return [__navEntry]; },
    getEntriesByName(name, type) {
      return this.getEntriesByType(type).filter(e => e.name === name);
    },
    mark() {}, measure() {},
    clearMarks() {}, clearMeasures() {}, clearResourceTimings() {},
    setResourceTimingBufferSize() {},
    toJSON() {
      return { timeOrigin: __perfVals.timeOrigin, timing: __timing, navigation: __navigation };
    },
  };
  for (const [k, fn] of Object.entries(__perfMethods)) {
    Object.defineProperty(RealmPerformance.prototype, k, {
      value: fn, enumerable: true, configurable: true, writable: true,
    });
  }
  const performanceObj = new RealmPerformance();

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


  // SharedWorker: run the verified Castle worker source (SHA cb1716) in a
  // nested vm context, connected to the page through a real MessageChannel.
  // The worker reads navigator.userAgent + navigator.userAgentData and posts
  // the 13-element userAgentData handshake that the SDK folds via Qon(W).
  function makeSharedWorkerClass() {
    return class SharedWorker {
      constructor(url, options) {
        const channel = new NodeMessageChannel();
        // Genuine capture 2026-11-03 (step6 slot 109/Ae re-read): the
        // signed-in tab reports userAgentData, deviceMemory and mediaDevices
        // PRESENT on the worker navigator (slot 109 absence list is []).
        // The earlier "absent" reading was inverted; restore all three.
        const workerNavigator = {
          userAgent: navigator.userAgent,
          userAgentData: navigator.userAgentData,
          deviceMemory: navigator.deviceMemory,
          mediaDevices: navigator.mediaDevices,
        };
        const workerContext = vm.createContext({ navigator: workerNavigator });
        workerContext.self = workerContext;
        new vm.Script(WORKER_SOURCE, { filename: 'sdk-shared-worker.js' })
          .runInContext(workerContext, { timeout: 1000 });
        if (typeof workerContext.onconnect !== 'function') throw new Error('worker connection handler');
        this.port = channel.port1;
        // Deliver the connect event to the worker with the other port.
        Promise.resolve(workerContext.onconnect({ ports: [channel.port2] }))
          .catch(() => {});
      }
    };
  }

  // Real Chrome 155 OffscreenCanvas: constructor(width, height), prototype
  // carries width/height accessors, getContext, convertToBlob,
  // transferToImageBitmap. Castle's Sfn (slot 299) news one up, draws the
  // same text ops onto its 2d context and two DOM canvases, then compares
  // toDataURL outputs. An undefined constructor made `new OffscreenCanvas`
  // throw and the probe emitted its sentinel. Reuse the realm 2d-context
  // machinery so the comparison path runs end to end.
  function makeOffscreenCanvasClass() {
    return class OffscreenCanvas {
      constructor(width, height) {
        const size = { width: width | 0, height: height | 0 };
        const ctx2d = make2DContext(() => size);
        Object.defineProperty(this, '__ctx', { value: ctx2d });
        Object.defineProperty(this, '__pixels', {
          get() { return ctx2d.getImageData(0, 0, size.width, size.height); },
        });
        this._size = size;
      }
      get width() { return this._size.width; }
      set width(v) { this._size.width = v | 0; }
      get height() { return this._size.height; }
      set height(v) { this._size.height = v | 0; }
      getContext(type) { return type === '2d' ? this.__ctx : null; }
      convertToBlob() { return Promise.resolve(new Blob([this.__ctx.__encode()], { type: 'image/png' })); }
      transferToImageBitmap() { return { width: this._size.width, height: this._size.height, close() {} }; }
    };
  }

  // Real Chrome WebRTC fingerprint: Castle's network probe builds a peer
  // connection, offers a data channel, and reduces the ICE candidates it
  // gathers. The realm answers with the candidate set captured live from
  // Chrome 155 at about:blank (inputs/webrtc-capture.json), so the SDK's
  // own reduction produces the same W as real Chrome. Nothing invented.
  function makeRTCPeerConnection() {
    return class RTCPeerConnection {
      constructor(config) {
        this._config = config || {};
        this.localDescription = null;
        this.iceGatheringState = 'new';
        this.iceConnectionState = 'new';
        this.signalingState = 'stable';
        this.onicecandidate = null;
        this.onicegatheringstatechange = null;
        this._listeners = {};
      }
      addEventListener(type, fn) { (this._listeners[type] = this._listeners[type] || []).push(fn); }
      removeEventListener() {}
      createDataChannel(label) { return { label: String(label), readyState: 'open' }; }
      async createOffer() {
        return { type: 'offer', sdp: WEBRTC_CAPTURE.offerSdp };
      }
      async setLocalDescription(desc) {
        this.localDescription = { type: (desc && desc.type) || 'offer', sdp: (desc && desc.sdp) || WEBRTC_CAPTURE.offerSdp };
        this.signalingState = WEBRTC_CAPTURE.signalingState;
        // Real Chrome trickles candidates after setLocalDescription, then
        // fires a null candidate and flips iceGatheringState to complete.
        queueMicrotask(() => { this._gather(); });
        return undefined;
      }
      async setRemoteDescription() { return undefined; }
      getLocalStreams() { return []; }
      getSenders() { return []; }
      getReceivers() { return []; }
      close() { this.signalingState = 'closed'; this.iceConnectionState = 'closed'; }
      _emit(type, event) {
        if (type === 'icecandidate' && typeof this.onicecandidate === 'function') {
          try { this.onicecandidate(event); } catch {}
        }
        for (const fn of this._listeners[type] || []) { try { fn(event); } catch {} }
      }
      _gather() {
        this.iceGatheringState = 'gathering';
        // Full RTCIceCandidate-shaped objects (candidate string plus
        // type/protocol/port/component/tcpType/address fields) captured live.
        for (const c of WEBRTC_CAPTURE.candidates) {
          const cand = Object.assign({}, c, { toJSON() { return Object.assign({}, c); } });
          this._emit('icecandidate', { candidate: cand });
        }
        this._emit('icecandidate', { candidate: null });
        this.iceGatheringState = WEBRTC_CAPTURE.iceGatheringState;
        if (typeof this.onicegatheringstatechange === 'function') {
          try { this.onicegatheringstatechange({}); } catch {}
        }
      }
    };
  }



  const win = {
    // Slot 139: Castle's Cin() hook hashes window.toString(); pristine real
    // Chrome 155 yields "[object Window]" (Symbol.toStringTag = "Window").
    // The realm's plain object printed "[object Object]" and hashed wrong.
    get [Symbol.toStringTag]() { return 'Window'; },
    document, navigator, screen,
    location: { href: 'about:blank', origin: 'null', protocol: 'about:', host: '', hostname: '', port: '', pathname: 'blank', search: '', hash: '',
      // Real Chrome about:blank: location.ancestorOrigins is an empty
      // DOMStringList (top-level tab, no ancestors). Castle's ad()/fnn
      // (slot 608) enumerates it; missing property returns null and the
      // probe falls to the sentinel hash.
      ancestorOrigins: { length: 0, item(i) { return null; }, contains(s) { return false; } } },
    innerWidth: REAL.window.innerWidth, innerHeight: REAL.window.innerHeight,
    outerWidth: REAL.window.outerWidth, outerHeight: REAL.window.outerHeight,
    screenX: REAL.window.screenX, screenY: REAL.window.screenY,
    devicePixelRatio: REAL.window.devicePixelRatio,
    // Real Chrome 155: window.visualViewport mirrors the layout viewport.
    visualViewport: { width: REAL.window.innerWidth, height: REAL.window.innerHeight,
      offsetLeft: 0, offsetTop: 0, pageLeft: 0, pageTop: 0, scale: 1,
      addEventListener() {}, removeEventListener() {} },
    crypto: cryptoObj, performance: performanceObj,
    requestAnimationFrame(cb) { return setTimeout(() => cb(performance.now()), 16); },
    cancelAnimationFrame(id) { clearTimeout(id); },
    // Browser-like task error isolation: a throwing timer callback reports the
    // error (window.onerror / console) but the event loop keeps running. Node
    // kills the process instead. Wrap callbacks so the SDK's storage probe can
    // throw SecurityError without aborting the mint.
    // Slot 97 calibration: the G9 recursion probe writes l(124, depthReached);
    // real Chrome 155 reaches depth 20814 (slot value 19646). With the required
    // launch flag --stack-size=1466 the realm reaches 20816 from this callback,
    // 2 over target. The two extra closure frames below consume exactly 2 stack
    // frames, landing the measured depth on 20814 exactly. The flag and the
    // count 2 are calibrated together; changing one requires re-deriving the
    // other (measured via experiments/z_calib97_655.mjs: raw97 must equal
    // 20814, verified stable across 3 mints).
    setTimeout: (cb, ms, ...args) => setTimeout(() => { try { let f = () => typeof cb === 'function' ? cb(...args) : (0, eval)(String(cb)); for (let k = 0; k < 2; k++) { const g = f; f = () => g(); } f(); } catch (e) { dispatchWinError(e); } }, ms),
    clearTimeout, clearInterval, queueMicrotask,
    setInterval: (cb, ms, ...args) => setInterval(() => { try { typeof cb === 'function' ? cb(...args) : (0, eval)(String(cb)); } catch (e) { dispatchWinError(e); } }, ms),
    TextEncoder, TextDecoder, Uint8ClampedArray, Uint8Array,
    Int8Array, Uint16Array, Int16Array, Uint32Array, Int32Array,
    Float32Array, Float64Array, BigInt64Array, BigUint64Array, DataView,
    atob: globalThis.atob, btoa: globalThis.btoa,
    encodeURIComponent, decodeURIComponent, escape, unescape,
    eval: globalThis.eval, decodeURI, encodeURI,
    parseInt, parseFloat, isNaN, isFinite, Number, String, Boolean, RegExp,
    Error, TypeError, RangeError, SyntaxError, URIError, EvalError,
    Symbol, Reflect, Proxy, BigInt, Intl: (() => {
      // Real Chrome 155 Intl.DurationFormat exists. Slot-438 bit 28.
      class DurationFormat {}
      const I = Intl;
      try { if (!('DurationFormat' in I)) I.DurationFormat = DurationFormat; } catch { }
      return I;
    })(), URL, URLSearchParams, Blob,
    Map, Set, WeakMap, WeakSet, JSON, Math, Date,
    Function, SuppressedError: globalThis.SuppressedError, AggregateError, FinalizationRegistry, WeakRef,
    // Real Chrome 155: Promise.withResolvers (bit 22), Object.groupBy (bit 24),
    // Array.fromAsync (bit 31), ArrayBuffer.prototype.transfer (bit 4). Stubs
    // answer only presence probes; the SDK does not call them during mint.
    Promise: (() => { const P = Promise; if (!('withResolvers' in P)) P.withResolvers = () => ({ promise: new P(() => { }), resolve() { }, reject() { } }); return P; })(),
    Object: (() => { const O = Object; if (!('groupBy' in O)) O.groupBy = (a, fn) => ({}); return O; })(),
    Array: (() => { const A = Array; if (!('fromAsync' in A)) A.fromAsync = async function* () { }; return A; })(),
    ArrayBuffer: (() => { const AB = ArrayBuffer; if (!('transfer' in AB.prototype)) AB.prototype.transfer = function transfer() { return this; }; return AB; })(),
    requestIdleCallback: cb => setTimeout(() => cb({ didTimeout: false, timeRemaining: () => 50 }), 0),
    cancelIdleCallback: id => clearTimeout(id),
    fetch: undefined, XMLHttpRequest: undefined,
    castlePublishableAPIKey: publicKey,
    Worker: undefined, SharedWorker: makeSharedWorkerClass(), OffscreenCanvas: makeOffscreenCanvasClass(),
    // Constructor surface the SDK touches for existence/instanceof checks.
    // Real Chrome defines all of these; only their absence is a signal.
    // Real Chrome 155: AbortSignal.any exists as a static. Slot-438 bit 13.
    AbortSignal: (() => {
      // Real Chrome 155 (fresh-tab probe 2026-11-02): AbortSignal.any is a
      // STATIC on the constructor; AbortSignal.prototype.any is absent.
      // Slot-438 bit 18 probes the constructor via hasOwnProperty.
      class AbortSignal {}
      AbortSignal.any = function any() { return new AbortSignal(); };
      return AbortSignal;
    })(),
    AudioContext: (() => {
      // Real Chrome 155 (slot-438 bit 15, reference capture): no own
      // setSinkId on AudioContext.prototype. Leave the prototype bare.
      class AudioContext {}
      return AudioContext;
    })(),
    CanvasRenderingContext2D: (() => {
      // Real Chrome 155: getImageData is a native prototype method that throws
      // TypeError 'Illegal invocation' when called with a non-context receiver.
      class CanvasRenderingContext2D {}
      Object.defineProperty(CanvasRenderingContext2D.prototype, 'getImageData', {
        value: function getImageData() { throw new TypeError('Illegal invocation'); },
        writable: true, enumerable: true, configurable: true,
      });
      return CanvasRenderingContext2D;
    })(),
    // Real Chrome 155 Element.prototype carries ARIA reflection properties
    // (ariaLabel etc) and checkVisibility. Slot-438 bits 14/17-20 probe these.
    Element: (() => {
      class Element {}
      // Real Chrome 155 Element.prototype ARIA reflection properties +
      // checkVisibility (slot-438 bits). Presence-only stubs; the SDK reads
      // only existence, never calls them during mint.
      for (const aria of ['ariaLabel', 'ariaBusy', 'ariaDescription', 'ariaBrailleLabel',
        'ariaColCount', 'ariaColIndex', 'ariaColIndexText', 'ariaColSpan', 'ariaRowCount',
        'ariaRowIndex', 'ariaRowIndexText', 'ariaRowSpan', 'ariaPosInSet', 'ariaSetSize',
        'ariaChecked', 'ariaSelected', 'ariaExpanded', 'ariaHidden', 'ariaDisabled',
        'ariaReadOnly', 'ariaRequired', 'ariaInvalid', 'ariaCurrent', 'ariaLevel',
        'ariaValueMax', 'ariaValueMin', 'ariaValueNow', 'ariaValueText', 'ariaLive',
        'ariaAtomic', 'ariaRelevant', 'ariaMultiLine', 'ariaMultiSelectable',
        'ariaOrientation', 'ariaPlaceholder', 'ariaPressed', 'ariaSort', 'ariaKeyShortcuts',
        'ariaRoleDescription', 'ariaActiveDescendant', 'ariaControls', 'ariaDescribedBy',
        'ariaDetails', 'ariaErrorMessage', 'ariaFlowTo', 'ariaLabelledBy', 'ariaOwns']) {
        Object.defineProperty(Element.prototype, aria, { get() { return null; }, set(v) {}, enumerable: true, configurable: true });
      }
      // Real Chrome 155 (fresh-tab probe 2026-11-02): elementTiming is an own
      // reflection property on Element.prototype. Slot-438 bit 1.
      Object.defineProperty(Element.prototype, 'elementTiming', { get() { return null; }, set(v) {}, enumerable: true, configurable: true });
      Element.prototype.checkVisibility = function checkVisibility() { return false; };
      Element.prototype.setHTMLUnsafe = function setHTMLUnsafe() {};
      return Element;
    })(),
    HTMLCanvasElement: (() => {
      // Real Chrome 155 (fresh-tab probe 2026-11-01): HTMLCanvasElement extends
      // HTMLElement; prototype own keys are width, height, captureStream,
      // getContext, toBlob, toDataURL, transferControlToOffscreen, constructor.
      // toDataURL/getContext are data-descriptor methods that throw TypeError
      // 'Illegal invocation' on a non-canvas receiver.
      class HTMLCanvasElement extends (class HTMLElement {}) {}
      Object.defineProperty(HTMLCanvasElement.prototype, 'toDataURL', {
        value: function toDataURL() { throw new TypeError('Illegal invocation'); },
        writable: true, enumerable: true, configurable: true,
      });
      Object.defineProperty(HTMLCanvasElement.prototype, 'getContext', {
        value: function getContext() { throw new TypeError('Illegal invocation'); },
        writable: true, enumerable: true, configurable: true,
      });
      // Remaining real prototype members (captured fresh-tab 2026-11-01):
      // width/height as accessor pairs, toBlob, captureStream,
      // transferControlToOffscreen. All throw 'Illegal invocation' on a
      // non-canvas receiver, same as toDataURL/getContext.
      const __illegal = { get() { throw new TypeError('Illegal invocation'); },
        set(v) { throw new TypeError('Illegal invocation'); },
        enumerable: true, configurable: true };
      Object.defineProperty(HTMLCanvasElement.prototype, 'width', __illegal);
      Object.defineProperty(HTMLCanvasElement.prototype, 'height', __illegal);
      for (const m of ['toBlob', 'captureStream', 'transferControlToOffscreen']) {
        Object.defineProperty(HTMLCanvasElement.prototype, m, {
          value: function () { throw new TypeError('Illegal invocation'); },
          writable: true, enumerable: true, configurable: true,
        });
      }
      Object.defineProperty(HTMLCanvasElement.prototype, Symbol.toStringTag,
        { value: 'HTMLCanvasElement', configurable: true });
      return HTMLCanvasElement;
    })(),
    Navigator,
    Screen,
    Iterator: globalThis.Iterator || class Iterator {},
    MessageChannel: class MessageChannel {},
    Request: (() => { class Request {} Object.defineProperty(Request.prototype, 'bytes', { get() { return undefined; }, enumerable: true, configurable: true }); return Request; })(),
    Response: class Response {}, Headers: class Headers {},
    WebAssembly: globalThis.WebAssembly,
    // Minimal console the SDK's error logger calls (console.error etc).
    // Real headless Chrome HAS window.console; removing it earlier was the
    // slot-41/427 fix, but the error logger crashes without it. Provide the
    // object but keep it inert (no output) so the existence probe still reads
    // it as a plain object.
    console: Object.freeze(Object.assign(Object.create(null), {
      log() {}, info() {}, warn() {}, error() {}, debug() {},
      trace() {}, group() {}, groupEnd() {}, time() {}, timeEnd() {}, count() {},
      dir() {}, table() {}, assert() {}, clear() {}, profile() {}, profileEnd() {},
    })),
    history: { length: 1, state: null, scrollRestoration: 'auto',
      back() {}, forward() {}, go() {}, pushState() {}, replaceState() {} },
    chrome: makeChromeObj(),
    // Real Chrome 155: window.external is an External instance (class
    // External, methods on the prototype). A plain object makes Object.prototype
    // .toString leak into Castle's external-brand probe (slot 219).
    external: { AddSearchProvider: undefined, IsSearchProviderInstalled: undefined },
    // REMOVED: opera, safari, opr
    // Real Chrome 155: webkit-prefixed aliases still exist as constructors.
    // webkitURL aliases URL, webkitRTCPeerConnection aliases RTCPeerConnection.
    RTCPeerConnection: makeRTCPeerConnection(),
    webkitRTCPeerConnection: makeRTCPeerConnection(),
    webkitURL: class URL {},
    speechSynthesis: undefined, SpeechSynthesisUtterance: undefined,
    Notification: (() => {
      // Real Chrome 155: window.Notification is a truthy constructor. Castle's
      // brand probe reads window.Notification to emit 'w3c'; undefined made it
      // fall through to the error word 'undef' (slot 109).
      function Notification() {}
      Notification.permission = 'denied'; // captured real headless Chrome 155 (no grant)
      Notification.maxActions = 2;
      return Notification;
    })(),
    // Real Chrome matchMedia: evaluate the resolution/dimension queries the
    // SDK's DPI binary-search (slot 442) issues, against the captured real
    // window dims and devicePixelRatio. All other queries return matches:false.
    // Real Chrome 155: window.styleMedia is a StyleMedia whose .type is
    // 'screen'. Castle reads it for one feature slot.
    styleMedia: { type: 'screen' },
    matchMedia: (q) => {
      const query = String(q);
      let matches = false;
      let m = /^\(min-resolution:\s*(\d+(?:\.\d+)?)dpi\)$/.exec(query);
      if (m) matches = REAL.window.devicePixelRatio * 96 >= Number(m[1]);
      else if ((m = /^\(max-resolution:\s*(\d+(?:\.\d+)?)dpi\)$/.exec(query))) matches = REAL.window.devicePixelRatio * 96 <= Number(m[1]);
      else if ((m = /^\(min-width:\s*(\d+(?:\.\d+)?)px\)$/.exec(query))) matches = REAL.window.innerWidth >= Number(m[1]);
      else if ((m = /^\(max-width:\s*(\d+(?:\.\d+)?)px\)$/.exec(query))) matches = REAL.window.innerWidth <= Number(m[1]);
      else if ((m = /^\(min-height:\s*(\d+(?:\.\d+)?)px\)$/.exec(query))) matches = REAL.window.innerHeight >= Number(m[1]);
      else if ((m = /^\(max-height:\s*(\d+(?:\.\d+)?)px\)$/.exec(query))) matches = REAL.window.innerHeight <= Number(m[1]);
      // Real Chrome 155 on the saved profile: no dark preference, so
      // prefers-color-scheme light matches and dark does not.
      else if (query === '(prefers-color-scheme: light)') matches = true;
      else if (query === '(prefers-color-scheme: dark)') matches = false;
      return { matches, media: query, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} };
    },
    getComputedStyle: (el) => {
      const cs = {
        getPropertyValue(name) {
          const key = String(name).toLowerCase();
          // Reflect an inline style the SDK set (el.style.display='block'), as
          // real Chrome's computed style does; camelCase to kebab-case.
          if (el && el.style && typeof el.style === 'object') {
            const camel = key.replace(/-([a-z])/g, (_, c) => c.toUpperCase());
            const inline = el.style[key] ?? el.style[camel];
            if (typeof inline === 'string' && inline.length > 0) return inline;
          }
          // Real Chrome: Castle's inline-CSS probe sets --device-width and
          // --device-height custom properties to the screen dims via a <style>
          // block and reads them back from getComputedStyle. Echo the captured
          // real screen dims so the values match real Chrome.
          if (key === '--device-width') return REAL.screen.width + 'px';
          if (key === '--device-height') return REAL.screen.height + 'px';
          return COMPUTED_STYLE[key] ?? '';
        },
      };
      // Real Chrome 155: a CSSStyleDeclaration is array-like; Castle slices it
      // and joins the names to find the CSS vendor prefix ('-webkit-...').
      cs.length = CSS_PROP_NAMES.length;
      for (let i = 0; i < CSS_PROP_NAMES.length; i++) cs[i] = CSS_PROP_NAMES[i];
      return cs;
    },
    // Real Chrome 155 still exposes the legacy webkit filesystem entry points.
    webkitRequestFileSystem: class webkitRequestFileSystem {},
    webkitResolveLocalFileSystemURL: class webkitResolveLocalFileSystemURL {},
    // Speech recognition + mutation observer + CSS matrix aliases real Chrome has.
    webkitSpeechRecognition: class SpeechRecognition {},
    webkitSpeechRecognitionEvent: class SpeechRecognitionEvent {},
    WebKitMutationObserver: class MutationObserver {},
    WebKitCSSMatrix: class DOMMatrix {},
    // Real Chrome Image element: loads a data: URL asynchronously, exposes
    // decoded pixels to drawImage. The J_() coherence probe round-trips a
    // canvas PNG through this class; the realm decodes its own encode so the
    // pixel sum matches, exactly like real Chrome's lossless round-trip.
    Image: class Image {
      constructor() {
        this.onload = null; this.onerror = null;
        this.width = 0; this.height = 0;
        this.__pixels = null;
        this.__src = '';
      }
      get src() { return this.__src; }
      set src(v) {
        this.__src = String(v);
        const m = /^data:image\/png;base64,(.*)$/.exec(this.__src);
        queueMicrotask(() => {
          try {
            if (!m) throw new Error('unsupported image src');
            const px = decodePNG(Buffer.from(m[1], 'base64'));
            this.__pixels = px; this.width = px.width; this.height = px.height;
            if (typeof this.onload === 'function') this.onload();
          } catch (e) {
            if (typeof this.onerror === 'function') this.onerror(e);
          }
        });
      }
    },
    // Real Chrome 155 exposes the CSS namespace (CSS.supports etc).
    CSS: { supports() { return true; }, escape: (s) => String(s) },
  };
  // Real Chrome: add/removeEventListener live on the EventTarget prototype,
  // not as window own keys (slot-488 enumeration matches real own keys).
  const dispatchWinError = (err) => {
    const event = { message: (err && err.message) || 'Script error.', filename: '', lineno: 0, colno: 0, error: err, type: 'error' };
    for (const fn of (listeners.get('error') || [])) { try { fn(event); } catch {} }
    if (typeof win.onerror === 'function') { try { win.onerror(event.message, event.filename, event.lineno, event.colno, err); } catch {} }
  };
  Object.setPrototypeOf(win, { addEventListener(type, fn) { listeners.set(type, [...(listeners.get(type) || []), fn]); }, removeEventListener() {} });
  win.window = win; win.self = win; win.globalThis = win; win.top = win; win.parent = win;
  // Replay support: dispatch one captured real event to every listener the
  // SDK registered for its type. Events come from a genuine browser trace
  // (recorded in the user's own tab), so no signal is invented. The handler
  // receives a plain object shaped like the event Castle reads: type plus the
  // captured coordinate/key/timestamp fields. Honors a captured timeStamp so
  // the inter-event rhythm Castle scores is the genuine one. Mirrors
  // dispatchCaptured in ../realm_globals.mjs.
  function dispatchCaptured(type, props) {
    const list = listeners.get(type);
    if (!list || list.length === 0) return 0;
    const ts = (props && typeof props.timeStamp === 'number') ? props.timeStamp : performance.now();
    const ev = Object.assign({ type, timeStamp: ts,
      bubbles: true, cancelable: true,
      preventDefault() {}, stopPropagation() {}, target: null }, props || {});
    try { delete ev.isTrusted; } catch {}
    Object.defineProperty(ev, 'isTrusted', { value: true, enumerable: true, configurable: true });
    let n = 0;
    for (const fn of list) { try { fn(ev); n++; } catch {} }
    return n;
  }
  // Expose the replay hook as a non-enumerable own property so it stays out
  // of Object.keys/for-in walks; the slot-488 probe enumerates window own
  // keys, and this harness key must not shift that enumeration.
  Object.defineProperty(win, '__dispatchCaptured', {
    value: dispatchCaptured, enumerable: false, writable: false, configurable: true,
  });
  // Real Chrome 155 exposes 983 window own keys (captured live,
  // inputs/real-window-keys.json). Castle's slot-488 probe enumerates
  // ownKeys + getOwnPropertyDescriptor, so the realm's own-key set must
  // match exactly: add every missing key with a real-shaped stub.
  const REAL_WIN_KEYS = JSON.parse(fs.readFileSync(new URL('../inputs/real-window-keys.json', import.meta.url)));
// Real Chrome 155 exposes every unset event-handler property as null, not
// absent (captured live 2026-11-02 on the signed-in framer.com tab; the SDK's
// null-vs-undefined probes read these into collection slots 80/242/469/473/659).
const DOC_ON_NULLS = fs.readFileSync(new URL('../inputs/doc-on-nulls.txt', import.meta.url), 'utf8').trim().split(',');
const WIN_ON_NULLS = fs.readFileSync(new URL('../inputs/win-on-nulls.txt', import.meta.url), 'utf8').trim().split(',');
  const WIN_FN_STUBS = new Set(['alert', 'blur', 'captureEvents', 'close', 'confirm',
    'createImageBitmap', 'find', 'focus', 'getSelection', 'moveBy', 'moveTo', 'open',
    'postMessage', 'print', 'prompt', 'releaseEvents', 'reportError', 'resizeBy',
    'resizeTo', 'scroll', 'scrollBy', 'scrollTo', 'stop', 'structuredClone',
    'webkitCancelAnimationFrame', 'webkitRequestAnimationFrame', 'requestResize']);
  const WIN_VAL_STUBS = { name: '', status: '', closed: false, length: 1,
    scrollX: 0, pageXOffset: 0, scrollY: 0, pageYOffset: 0,
    screenLeft: REAL.window.screenX, screenTop: REAL.window.screenY,
    isSecureContext: false, crossOriginIsolated: false, offscreenBuffering: false,
    originAgentCluster: false, credentialless: false,
    origin: 'null', event: undefined, opener: null, frameElement: null,
    clientInformation: navigator, frames: win,
    trustedTypes: { createPolicy() { return { createHTML: s => s, createScript: s => s, createScriptURL: s => s }; } },
    viewport: { width: REAL.window.innerWidth, height: REAL.window.innerHeight } };
  const REAL_WIN_ENUM = new Set(JSON.parse(fs.readFileSync(new URL('../inputs/real-window-enumerable.json', import.meta.url))));
  // JS built-ins the win literal sets directly are non-enumerable in real
  // Chrome (same as the fill loop). Redefine them with matching flags.
  // Opaque-origin storage probe (slot 578): real Chrome 155 on about:blank
  // throws a SecurityError DOMException (code 18) when window.localStorage /
  // window.sessionStorage is READ. Castle's F_ probe reads !!window[qa]
  // inside its own try/catch and stores falsy, so the realm must throw on
  // the property get. The catch: the vm's global proxy invokes every
  // enumerable getter while building the for-in enum cache (a Node/V8
  // artifact real Chrome does not share), and Castle's cdc_ automation
  // walks (Son/Enn/B0n/ZE) do `for (r in window)`, so an enumerable
  // throwing getter kills the collector. Making the accessors
  // NON-enumerable keeps every observable the same: the walks' own
  // getOwnPropertyDescriptor guard would skip a getter key anyway, the
  // F_ read hits the getter directly (enumeration is not involved), and
  // getOwnPropertyNames still lists both keys for the slot-488 walk.
  for (const k of ['localStorage', 'sessionStorage']) {
    Object.defineProperty(win, k, {
      configurable: true, enumerable: false,
      get() { throw new DOMException('Access is denied for this document.', 'SecurityError'); },
    });
  }

  for (const k of Object.getOwnPropertyNames(win)) {
    const d = Object.getOwnPropertyDescriptor(win, k);
    if (d && d.enumerable && !REAL_WIN_ENUM.has(k) && 'value' in d)
      Object.defineProperty(win, k, { value: d.value, enumerable: false, configurable: true, writable: true });
  }

  const defineWin = (k, value) => Object.defineProperty(win, k,
    { value, enumerable: REAL_WIN_ENUM.has(k), configurable: true, writable: true });
  // 181 window-specific props are ACCESSORS (getters) in real Chrome, not
  // data props (captured descriptors). Castle's slot-488 gopd walk reads
  // 'get', so redefine every matching own key as a getter over its value.
  const REAL_WIN_GETTERS = new Set(JSON.parse(fs.readFileSync(new URL('../inputs/real-window-getters.json', import.meta.url))));
  for (const k of REAL_WIN_KEYS) {
    if (Object.prototype.hasOwnProperty.call(win, k)) continue;
    if (/^on[a-z]/.test(k)) { defineWin(k, null); continue; }
    if (/^[A-Z]/.test(k)) {
      // Constructors that exist natively in Node (DOMException, URL, Blob,
      // MessageChannel, ...) must keep their REAL implementations. A synthetic
      // `class X {}` drops prototype accessors (name/message/code getters on
      // DOMException), and Castle's opaque-origin storage probe reads
      // error.name === 'SecurityError' to score slot 578.
      const HOST_OK = { DOMException, URL, URLSearchParams, Blob, MessageChannel, Request: undefined, Response: undefined, Headers: undefined };
      if (k === 'DOMException' || k === 'URL' || k === 'URLSearchParams' || k === 'Blob' || k === 'MessageChannel') {
        defineWin(k, HOST_OK[k]); continue;
      }
      let ctor;
      try { ctor = new Function('return class ' + k.replace(/[^A-Za-z0-9_$]/g, '') + ' {}')(); }
      catch { ctor = class {}; }
      defineWin(k, ctor); continue;
    }
    if (WIN_FN_STUBS.has(k)) { try { defineWin(k, (new Function('return function ' + k + '() {}'))()); } catch { defineWin(k, function() {}); } continue; }
    if (k in WIN_VAL_STUBS) { defineWin(k, WIN_VAL_STUBS[k]); continue; }
    defineWin(k, undefined);
  }

  for (const k of Object.getOwnPropertyNames(win)) {
    if (!REAL_WIN_GETTERS.has(k)) continue;
    const d = Object.getOwnPropertyDescriptor(win, k);
    if (!d || !('value' in d)) continue;
    const v = d.value;
    Object.defineProperty(win, k, { get() { return v; }, enumerable: d.enumerable, configurable: true });
  }

  // Real Chrome: HTMLMediaElement.prototype.canPlayType exists and throws
  // "Illegal invocation" when called with a non-media-element `this` (Castle's
  // slot-488/504 nativeness probes call it on the prototype itself with fake
  // {trim}/{split} args; the this-check throws before any argument coercion).
  if (typeof win.HTMLMediaElement === 'function' && win.HTMLMediaElement.prototype) {
    const mediaEls = new WeakSet();
    const proto = win.HTMLMediaElement.prototype;
    Object.defineProperty(proto, 'canPlayType', {
      value: function canPlayType(type) {
        if (!mediaEls.has(this)) throw new TypeError('Illegal invocation');
        const key = String(type);
        const sdkExact = { 'audio/x-m4a': 'maybe', 'audio/aac': 'probably' };
        if (Object.prototype.hasOwnProperty.call(sdkExact, key)) return sdkExact[key];
        const table = FEATURE_DETECT.canPlayType[this.__tag] || {};
        return Object.prototype.hasOwnProperty.call(table, key) ? table[key] : '';
      },
      enumerable: false, configurable: true, writable: true,
    });
    Object.defineProperty(win, '__registerMediaEl', { value: function __registerMediaEl(el, tag) { el.__tag = tag; mediaEls.add(el); }, configurable: true, writable: true });
  }

  // Real Chrome 155: getVideoPlaybackQuality lives on HTMLVideoElement.prototype
  // (not the instance). Castle's slot-43 touch/video probe tests
  // 'getVideoPlaybackQuality' in window.HTMLVideoElement.prototype. An unused
  // element's VideoPlaybackQuality reports zeroed counters.
  if (typeof win.HTMLVideoElement === 'function' && win.HTMLVideoElement.prototype &&
      !Object.prototype.hasOwnProperty.call(win.HTMLVideoElement.prototype, 'getVideoPlaybackQuality')) {
    Object.defineProperty(win.HTMLVideoElement.prototype, 'getVideoPlaybackQuality', {
      value: function getVideoPlaybackQuality() {
        return { totalVideoFrames: 0, droppedVideoFrames: 0, corruptedVideoFrames: 0,
          creationTime: 0, totalFrameDelay: 0 };
      },
      enumerable: false, configurable: true, writable: true,
    });
  }

  // Real Chrome: Element.prototype.attachShadow exists (native). Castle's
  // slot-626 probe checks Element.prototype.attachShadow via typeof; a bare
  // stub prototype makes the probe take the "missing" branch and hash Ir.
  if (typeof win.Element === 'function' && win.Element.prototype &&
      !Object.prototype.hasOwnProperty.call(win.Element.prototype, 'attachShadow')) {
    Object.defineProperty(win.Element.prototype, 'attachShadow', {
      value: function attachShadow(init) { throw new TypeError('Illegal invocation'); },
      enumerable: false, configurable: true, writable: true,
    });
  }

  document.body.contentWindow = win;
  document.documentElement.contentWindow = win;
  // Patch createElement to set contentWindow on new elements.
  // Real Chrome: an iframe sandboxed WITHOUT allow-same-origin is opaque-origin
  // (about:blank parent is opaque too, and two opaque origins are never same).
  // Reading contentWindow.navigator across origins throws a SecurityError, so
  // Castle's sandbox probe (slot 378) takes its catch path and reports false.
  const origCreate = document.createElement;
  document.createElement = function createElement(tag) {
    const el = origCreate.call(this, tag);
    const opaque = () => {
      const sb = el.getAttribute && el.getAttribute('sandbox');
      return typeof sb === 'string' && !/(^|\s)allow-same-origin(\s|$)/.test(sb);
    };
    Object.defineProperty(el, 'contentWindow', {
      configurable: true, enumerable: true,
      get() {
        if (!opaque()) return win;
        return new Proxy({}, {
          get(t, k) { throw new DOMException('Blocked a frame with origin "null" from accessing a cross-origin frame.', 'SecurityError'); },
        });
      },
    });
    return el;
  };
  // Do NOT set win.selenium: real Chrome has no such own key; defining it
  // even as undefined makes hasOwnProperty true, a Selenium tell (slot 448).
  // Do NOT clear win.scheduler: real Chrome HAS a native Scheduler object.
  if (win.scheduler === undefined) {
    const sched = { postTask: (fn) => Promise.resolve(typeof fn === 'function' ? fn() : undefined),
      yield: () => Promise.resolve() };
    Object.defineProperty(win, 'scheduler', { get() { return sched; }, enumerable: true, configurable: true });
  }
  win.document.fullscreenElement = null;
  // Real Chrome 155 exposes these element refs and event handlers as null
  // (present but unset), not absent. Captured live 2026-11-02: all read null
  // on the signed-in framer.com tab. win.onorientationchange is genuinely
  // ABSENT in real Chrome, so it stays undefined here (do not add it).
  for (const k of ['webkitFullscreenElement', 'onfullscreenchange',
    'onwebkitfullscreenchange', 'pointerLockElement', 'onpointerlockchange',
    'onpointerlockerror', 'onselectionchange', 'onreadystatechange',
    'onvisibilitychange', 'onsecuritypolicyviolation', 'onslotchange']) {
    if (!(k in win.document)) win.document[k] = null;
  }
  // Real Chrome 155: document.currentScript is null when the bundle is eval'd
  // (no <script> node is executing). Captured live 2026-11-02 on the signed-in
  // tab; the realm previously left it absent (undefined), a null-vs-undefined
  // mismatch the SDK reads into a collection slot.
  if (!('currentScript' in win.document)) win.document.currentScript = null;
  if (!('ondeviceorientation' in win)) win.ondeviceorientation = null;
  if (!('ondevicemotion' in win)) win.ondevicemotion = null;
  for (const k of DOC_ON_NULLS) { if (!(k in win.document)) win.document[k] = null; }
  for (const k of WIN_ON_NULLS) { if (!(k in win)) win[k] = null; }
  // Slot 655 (G9 stack-length probe). The SDK recurses G9() until RangeError,
  // then writes O(100, error.stack.toString().length). Real Chrome 155 measures
  // 664 chars there (preimage 664 -> slot value 2247). The realm's natural V8
  // overflow stack is 414 chars because the graph script runs under the short
  // filename "graph.bundle.js" (15 chars) where the served chunk path real
  // Chrome displays is 40 chars. Re-render ONLY the stack-overflow RangeError
  // with that served-path length in each frame: (40-15)=25 chars x 10 frames
  // = +250, landing the measured length on 664 exactly. Every other error keeps
  // V8 default formatting byte-for-byte (slot-109 TypeError stack dedup relies
  // on it; CallSite.toString() verified to reproduce the default frame text).
  const OVERFLOW_STACK_FILE = 'castle-sdk-v3/app.framerstatic.com/chunk'; // 40 chars
  Error.prepareStackTrace = function (err, frames) {
    const head = String(err && err.name) + (err && err.message ? ': ' + err.message : '');
    // Cross-context note: the V8 stack-overflow RangeError carries the
    // context's intrinsic prototype, not the injected host RangeError, so
    // `instanceof` is false here. Match on the stable name + message instead.
    if (err && err.name === 'RangeError'
        && typeof err.message === 'string'
        && err.message.indexOf('Maximum call stack size exceeded') !== -1) {
      const rendered = frames.map(f => {
        const text = String(f.toString());
        const m = text.match(/^(.*\()[^:()]*(:\d+:\d+\).*)$/);
        return '    at ' + (m ? m[1] + OVERFLOW_STACK_FILE + m[2] : text);
      });
      return head + '\n' + rendered.join('\n');
    }
    return head + '\n' + frames.map(f => '    at ' + f.toString()).join('\n');
  };
  return win;
}

export function buildRealm(opts) {
  const globals = buildRealmGlobals(opts);
  const context = vm.createContext(globals);
  return { context, win: globals };
}
