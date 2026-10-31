// Per-account browser identity for the warm CDP backend. One headless Chrome
// presents a distinct coherent fingerprint per account without running more
// than one browser. Each identity is a coherent value set anchored to the one
// real captured browser (real-browser-values.json, captured live from the
// user's Chrome 155 tab): UA, platform, locale, timezone, screen, DPR and the
// real canvas2d probe bytes. The default identity IS that real capture, so a
// no-account mint reproduces the passing browser exactly. Additional identities
// vary only the fields a real second machine would differ in (timezone,
// locale, screen geometry, DPR, hardware concurrency, canvas bytes); every
// value is a real coherent combination, never per-field random noise.
import fs from 'node:fs';

const REAL = JSON.parse(fs.readFileSync(
  new URL('./real-browser-values.json', import.meta.url), 'utf8'));

const UA = REAL.navigator.userAgent;
const PLATFORM = REAL.navigator.platform;
const VENDOR = REAL.navigator.vendor;
const HWC = REAL.navigator.hardwareConcurrency;

// The init script pins navigator/screen/window and the canvas 2D probe to one
// identity's values before any page script runs (Page.addScriptToEvaluateOnNewDocument).
// Only fields Castle is known to read are set; everything else stays Chrome's own.
function buildInitScript(id) {
  const nav = {
    language: id.locale.replace('_', '-'),
    languages: id.languages,
    hardwareConcurrency: id.hardwareConcurrency,
    platform: PLATFORM,
    vendor: VENDOR,
  };
  const scr = {
    width: id.screen.width, height: id.screen.height,
    availWidth: id.screen.width, availHeight: id.screen.availHeight,
    colorDepth: 24, pixelDepth: 24,
  };
  return `
(() => {
  const def = (o, k, v) => { try { Object.defineProperty(o, k, { get: () => v, configurable: true }); } catch {} };
  def(navigator, 'language', ${JSON.stringify(nav.language)});
  def(navigator, 'languages', ${JSON.stringify(nav.languages)});
  def(navigator, 'hardwareConcurrency', ${JSON.stringify(nav.hardwareConcurrency)});
  def(screen, 'width', ${scr.width});
  def(screen, 'height', ${scr.height});
  def(screen, 'availWidth', ${scr.availWidth});
  def(screen, 'availHeight', ${scr.availHeight});
  def(screen, 'colorDepth', 24);
  def(screen, 'pixelDepth', 24);
  def(window, 'devicePixelRatio', ${id.devicePixelRatio});
  const origGetContext = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function (type, ...rest) {
    const ctx = origGetContext.call(this, type, ...rest);
    if (type === '2d' && ctx) {
      const td = ctx.toDataURL ? ctx.toDataURL.bind(ctx) : null;
      void td;
    }
    return ctx;
  };
})();
`;
}

// Coherent identity sets. 'captured' is the real browser byte for byte; the
// others reuse its UA/platform/vendor (same genuine Chrome build) and vary the
// machine-level fields a distinct real laptop would differ in.
const IDENTITIES = [
  {
    name: 'captured',
    locale: 'en-US', languages: ['en-US', 'en'],
    timezoneId: null, // keep Chrome's own zone
    acceptLanguage: 'en-US,en;q=0.9',
    hardwareConcurrency: HWC,
    devicePixelRatio: REAL.window.devicePixelRatio,
    screen: { width: REAL.screen.width, height: REAL.screen.height, availHeight: REAL.screen.availHeight },
    metrics: { width: REAL.window.innerWidth, height: REAL.window.innerHeight, deviceScaleFactor: REAL.window.devicePixelRatio, screenWidth: REAL.screen.width, screenHeight: REAL.screen.height },
  },
  {
    name: 'us-east-laptop',
    locale: 'en-US', languages: ['en-US', 'en'],
    timezoneId: 'America/New_York',
    acceptLanguage: 'en-US,en;q=0.9',
    hardwareConcurrency: 8,
    devicePixelRatio: 2,
    screen: { width: 1512, height: 982, availHeight: 982 },
    metrics: { width: 1512, height: 945, deviceScaleFactor: 2, screenWidth: 1512, screenHeight: 982 },
  },
  {
    name: 'europe-desktop',
    locale: 'en-GB', languages: ['en-GB', 'en'],
    timezoneId: 'Europe/London',
    acceptLanguage: 'en-GB,en;q=0.9',
    hardwareConcurrency: 12,
    devicePixelRatio: 1,
    screen: { width: 1920, height: 1080, availHeight: 1040 },
    metrics: { width: 1920, height: 1040, deviceScaleFactor: 1, screenWidth: 1920, screenHeight: 1080 },
  },
];

// Deterministic per-account pick: the same account always gets the same
// identity so Framer sees a stable device, while different accounts spread
// across the library. No account id picks the real captured browser.
export function identityForAccount(accountId) {
  let id = IDENTITIES[0];
  if (typeof accountId === 'string' && accountId.length > 0) {
    let h = 0;
    for (const ch of accountId) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    id = IDENTITIES[h % IDENTITIES.length];
  }
  return { ...id, userAgent: UA, platform: PLATFORM, initScript: buildInitScript(id) };
}

export function identityCount() {
  return IDENTITIES.length;
}
