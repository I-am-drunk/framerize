// CDP mint backend for the production token service.
// One warm headless Chrome serves many mints (~200 MB once, not per request).
// Each mint gets a fresh disposable browser context (isolated storage) and a
// fresh page, so no state carries between accounts. The browser spawns once
// per service process and shuts down cleanly on exit or CDP failure.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import * as pins from '../pins.mjs';
import { pickTrace } from './interaction_traces.mjs';
import { identityForAccount } from './browser_identities.mjs';

const CHROME = '/home/tjninja/.local/share/lets-study-test-browser/chrome/opt/google/chrome/chrome';
const MINT_TIMEOUT_MS = 120000;
const CDP_PORT_BASE = 9400;

function get(port, p) {
  return new Promise((res, rej) => {
    http.get({ host: '127.0.0.1', port, path: p }, r => {
      let b = ''; r.on('data', c => b += c);
      r.on('end', () => { try { res(JSON.parse(b)); } catch (e) { rej(e); } });
    }).on('error', rej);
  });
}
const sleep = ms => new Promise(r => setTimeout(r, ms));

// The warm browser. Held in module state; spawned once, reused across mints.
// The lifecycle is: ensureBrowser() spawns if absent, mintOnceCdp() uses it,
// closeBrowser() kills the whole process group and removes the profile.
let browser = null; // { child, port, profile, ws, nextId, pending, sessionListeners }
let spawning = null;  // in-flight spawnBrowser() promise, so one browser only

function chromeArgs(port, profile) {
  return [
    '--headless=new', '--no-first-run', '--no-default-browser-check',
    '--disable-extensions', '--disable-sync', '--disable-background-networking',
    '--disable-component-update', '--mute-audio', '--no-sandbox',
    '--window-size=1280,800', '--force-device-scale-factor=1',
    '--disable-gpu', '--disable-software-rasterizer', '--disable-dev-shm-usage',
    '--disable-features=Translate,BackForwardCache,AcceptCHFrame,MediaRouter,OptimizationHints,InterestFeedContentSuggestions,CalculateNativeWinOcclusion,AudioServiceOutOfProcess',
    '--disable-breakpad', '--disable-crash-reporter', '--disable-logging', '--no-zygote',
    '--disable-renderer-backgrounding', '--disable-background-timer-throttling',
    '--disable-component-extensions-with-background-pages',
    '--disable-site-isolation-trials', '--renderer-process-limit=1',
    '--enable-features=MemoryPurgeOnBackground',
    '--in-process-gpu',
    '--js-flags=--max-old-space-size=128',
    '--remote-debugging-port=' + port,
    '--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36',
    '--user-data-dir=' + profile,
    'about:blank',
  ];
}

async function connectBrowserWs(port) {
  const v = await get(port, '/json/version');
  if (!v || !v.webSocketDebuggerUrl) throw new Error('no browser ws');
  const { default: WebSocket } = await import('../inputs/dom/node_modules/ws/wrapper.mjs');
  const ws = new WebSocket(v.webSocketDebuggerUrl, { perMessageDeflate: false, maxPayload: 64 * 1024 * 1024 });
  await new Promise((res, rej) => { ws.on('open', res); ws.on('error', rej); });
  return ws;
}

// Close the startup about:blank tab (and any browser_ui target) so the warm
// browser idles with zero renderers between mints. Mints create their own
// page target inside a disposable context, so nothing needed is lost.
async function closeStartupTargets(b) {
  try {
    const { targetInfos } = await bsend(b, 'Target.getTargets');
    for (const t of targetInfos) {
      if (t.type === 'page' || t.type === 'browser_ui') {
        await bsend(b, 'Target.closeTarget', { targetId: t.targetId }).catch(() => {});
      }
    }
  } catch {}
}

// Spawn Chrome detached (own process group) so the whole tree, renderers and
// gpu helpers included, dies on one group kill. Wait for the devtools port.
async function spawnBrowser() {
  const run = 'svc-' + process.pid + '-' + Math.random().toString(36).slice(2, 7);
  const port = CDP_PORT_BASE + (parseInt(run.replace(/\D/g, '').slice(-4) || '0', 10) % 400);
  const profile = '/tmp/framerize-cdp-profile-' + run;
  sweepStaleProfiles();
  const child = spawn(CHROME, chromeArgs(port, profile),
    { stdio: ['ignore', 'ignore', 'ignore'], detached: true });
  for (let i = 0; i < 80; i++) {
    try {
      const v = await get(port, '/json/version');
      if (v && v.Browser) {
        const ws = await connectBrowserWs(port);
        return { child, port, profile, ws, nextId: 0, pending: new Map(), sessionListeners: new Map() };
      }
    } catch {}
    await sleep(250);
  }
  try { process.kill(-child.pid, 'SIGKILL'); } catch { try { child.kill('SIGKILL'); } catch {} }
  throw new Error('chrome devtools not ready');
}

// Wire the browser-level CDP connection: id routing, event fan-out by session.
function wireBrowser(b) {
  b.ws.on('message', d => {
    let m; try { m = JSON.parse(d); } catch { return; }
    if (m.id && b.pending.has(m.id)) { b.pending.get(m.id)(m); b.pending.delete(m.id); return; }
    if (m.method && m.sessionId && b.sessionListeners.has(m.sessionId)) {
      for (const fn of b.sessionListeners.get(m.sessionId)) { try { fn(m); } catch {} }
    }
  });
  b.ws.on('close', () => {
    if (browser === b) browser = null;
    // Reject every in-flight command so a dead browser never wedges the
    // service: the mint's await throws, the retry path respawns fresh.
    for (const [, res] of b.pending) { try { res({ error: { message: 'browser socket closed' } }); } catch {} }
    b.pending.clear();
  });
  b.ws.on('error', () => {});
}

// Send a browser-level command (no session).
function bsend(b, method, params = {}) {
  return new Promise((res, rej) => {
    const id = ++b.nextId;
    b.pending.set(id, m => m.error ? rej(new Error(method + ': ' + JSON.stringify(m.error))) : res(m.result));
    b.ws.send(JSON.stringify({ id, method, params }));
  });
}

// Spawn the warm browser if it is gone or its socket died.
// Remove profile dirs left by any previous dead service run. A profile is
// stale when no live process has it open; only dirs under our prefix whose
// owner pid is gone are removed.
function sweepStaleProfiles() {
  let entries;
  try { entries = fs.readdirSync('/tmp', { withFileTypes: true }); } catch { return; }
  for (const e of entries) {
    if (!e.isDirectory() || !e.name.startsWith('framerize-cdp-profile-svc-')) continue;
    const m = e.name.match(/^framerize-cdp-profile-svc-(\d+)-/);
    if (!m) continue;
    const pid = parseInt(m[1], 10);
    if (pid === process.pid) continue;
    let alive = false;
    try { process.kill(pid, 0); alive = true; } catch {}
    if (alive) continue;
    try { fs.rmSync('/tmp/' + e.name, { recursive: true, force: true }); } catch {}
  }
}

async function ensureBrowser() {
  if (browser && browser.ws.readyState === 1) return browser;
  if (browser) await closeBrowser();
  if (!spawning) spawning = spawnBrowser();
  try { browser = await spawning; } finally { spawning = null; }
  wireBrowser(browser);
  installExitHook();
  await closeStartupTargets(browser);
  return browser;
}

// One exit hook for the whole service process. Registered once; always kills
// the current warm browser's process group. Registering inside ensureBrowser
// would stack one listener per recycle and trip MaxListeners over a long run.
let exitHookInstalled = false;
function installExitHook() {
  if (exitHookInstalled) return;
  exitHookInstalled = true;
  process.once('exit', () => {
    try { if (browser) process.kill(-browser.child.pid, 'SIGKILL'); } catch {}
  });
}

// Kill the whole process group, remove the profile, drop module state.
async function closeBrowser() {
  const b = browser; browser = null;
  if (!b) return;
  try { b.ws.close(); } catch {}
  try { process.kill(-b.child.pid, 'SIGKILL'); } catch { try { b.child.kill('SIGKILL'); } catch {} }
  // Wait briefly for the group to die so no child holds a profile file open,
  // then remove the profile. SIGKILL delivery is fast but the tree teardown
  // (renderer/gpu/utility exits) takes a beat.
  for (let i = 0; i < 20; i++) {
    try { process.kill(-b.child.pid, 0); await sleep(150); } catch { break; }
  }
  try { fs.rmSync(b.profile, { recursive: true, force: true }); } catch (e) {
    console.error('profile rm failed:', b.profile, e && e.message);
  }
}

// A mint session: one fresh page target in one fresh disposable context.
async function openMintSession(b, identity) {
  const { browserContextId } = await bsend(b, 'Target.createBrowserContext', { disposeOnDetach: true });
  const { targetId } = await bsend(b, 'Target.createTarget', { url: 'about:blank', browserContextId });
  if (!targetId) throw new Error('no page target');
  const { sessionId } = await bsend(b, 'Target.attachToTarget', { targetId, flatten: true });
  const listeners = new Map();
  b.sessionListeners.set(sessionId, listeners);
  return { browserContextId, targetId, sessionId, listeners };
}

// Close the page, drop listeners, dispose the isolated context.
async function closeMintSession(b, s) {
  b.sessionListeners.delete(s.sessionId);
  try { await bsend(b, 'Target.detachFromTarget', { sessionId: s.sessionId }); } catch {}
  try { await bsend(b, 'Target.closeTarget', { targetId: s.targetId }); } catch {}
  try { await bsend(b, 'Target.disposeBrowserContext', { browserContextId: s.browserContextId }); } catch {}
}

// Session-scoped command.
function ssend(b, s, method, params = {}) {
  return new Promise((res, rej) => {
    const id = ++b.nextId;
    b.pending.set(id, m => m.error ? rej(new Error(method + ': ' + JSON.stringify(m.error))) : res(m.result));
    b.ws.send(JSON.stringify({ id, method, params, sessionId: s.sessionId }));
  });
}

// One session event listener; returns an unsubscribe fn.
function onEvent(b, s, method, fn) {
  if (!b.sessionListeners.has(s.sessionId)) b.sessionListeners.set(s.sessionId, new Map());
  const set = b.sessionListeners.get(s.sessionId);
  const key = method + ':' + (set.size + 1);
  set.set(key, fn);
  return () => set.delete(key);
}

async function seval(b, s, expr, timeout = 30000) {
  const r = await ssend(b, s, 'Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true, timeout });
  if (r.exceptionDetails) throw new Error('eval: ' + JSON.stringify(r.exceptionDetails).slice(0, 300));
  return r.result ? r.result.value : undefined;
}

// Apply one per-account identity to the session: locale, timezone, screen and
// device metrics, and the init script that pins navigator/screen/canvas.
async function applyIdentity(b, s, identity) {
  if (identity.timezoneId) {
    await ssend(b, s, 'Emulation.setTimezoneOverride', { timezoneId: identity.timezoneId }).catch(() => {});
  }
  if (identity.locale) {
    await ssend(b, s, 'Emulation.setLocaleOverride', { locale: identity.locale }).catch(() => {});
  }
  const m = identity.metrics;
  if (m) {
    await ssend(b, s, 'Emulation.setDeviceMetricsOverride', {
      width: m.width, height: m.height,
      deviceScaleFactor: m.deviceScaleFactor, mobile: !!m.mobile,
      screenWidth: m.screenWidth, screenHeight: m.screenHeight,
    }).catch(() => {});
  }
  if (identity.userAgent) {
    await ssend(b, s, 'Network.setUserAgentOverride', {
      userAgent: identity.userAgent,
      acceptLanguage: identity.acceptLanguage || 'en-US',
      platform: identity.platform || 'Linux x86_64',
    }).catch(() => {});
  }
  await ssend(b, s, 'Page.addScriptToEvaluateOnNewDocument', { source: identity.initScript });
}

// Run one mint inside an open session at the editor URL. Loads the pinned SDK
// graph, sets the key, replays a randomly picked real trace, and mints.
async function runMint(b, s, editorUrl) {
  await ssend(b, s, 'Page.enable', {});
  await ssend(b, s, 'Runtime.enable', {});
  const loaded = new Promise(res => {
    const off = onEvent(b, s, 'Page.loadEventFired', () => { off(); res(); });
    setTimeout(() => { off(); res(); }, 15000);
  });
  await ssend(b, s, 'Page.navigate', { url: 'https://framer.com/projects' });
  await loaded;
  // framer.com registers service workers that would otherwise outlive the
  // mint page and accumulate across mints. Unregister them in this context.
  await seval(b, s, `(async () => {
    try {
      const regs = await navigator.serviceWorker.getRegistrations();
      await Promise.all(regs.map(r => r.unregister()));
      return regs.length;
    } catch (e) { return 0; }
  })()`).catch(() => {});
  if (editorUrl && editorUrl.startsWith('https://framer.com/projects/')) {
    await seval(b, s, `history.replaceState(null, '', ${JSON.stringify(editorUrl)}); location.pathname`);
  }
  const { buildGraphScript } = await import('./graph_script.mjs');
  const bundle = buildGraphScript({
    assets: pins.ASSETS, modules: pins.MODULES, cdn: pins.CDN,
    collectionTransform: null,
  });
  await seval(b, s, `(0,eval)(${JSON.stringify(bundle)}); 'graph-loaded'`);
  await seval(b, s, `window.castlePublishableAPIKey = ${JSON.stringify(pins.PUBLIC_KEY)}; 'key-set'`);
  const replayed = await replayTrace(b, s);
  const out = await seval(b, s, `(async () => {
    const m = globalThis.__modules.wrapper; if (!m || !m.a) return { err: 'no wrapper' };
    try { const tok = await m.a({ budget_ms: 8000 }); return { tok }; }
    catch (e) { return { err: String(e && e.message || e) }; }
  })()`, 20000);
  if (!out || !out.tok) throw new Error('mint: ' + JSON.stringify(out && out.err));
  return { token: out.tok, replayed };
}

// Replay one randomly picked real captured trace via the CDP Input domain.
// Events are trusted (isTrusted=true) and keep their genuine timing.
async function replayTrace(b, s) {
  const events = pickTrace();
  let prevDt = 0;
  for (const ev of events) {
    const gap = (typeof ev.dt === 'number' ? ev.dt : prevDt) - prevDt;
    if (gap > 0) await sleep(Math.min(gap, 1000));
    if (typeof ev.dt === 'number') prevDt = ev.dt;
    await dispatchInput(b, s, ev);
  }
  return events.length;
}

async function dispatchInput(b, s, ev) {
  const t = ev.type;
  const x = ev.clientX ?? ev.x ?? 0, y = ev.clientY ?? ev.y ?? 0;
  try {
    if (t === 'mousemove' || t === 'pointermove') {
      await ssend(b, s, 'Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, pointerType: 'mouse' });
    } else if (t === 'mousedown') {
      await ssend(b, s, 'Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1 });
    } else if (t === 'mouseup') {
      await ssend(b, s, 'Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 });
    } else if (t === 'wheel') {
      await ssend(b, s, 'Input.dispatchMouseEvent', { type: 'mouseWheel', x, y, deltaX: ev.deltaX ?? 0, deltaY: ev.deltaY ?? 0 });
    } else if (t === 'keydown' || t === 'keypress' || t === 'keyup') {
      await ssend(b, s, 'Input.dispatchKeyEvent', {
        type: t === 'keyup' ? 'keyUp' : 'rawKeyDown',
        key: ev.key ?? '', code: ev.code ?? '', text: t === 'keypress' ? (ev.key ?? '') : undefined,
      });
    }
  } catch {}
}

// Mint one genuine wire token on the warm browser. editorUrl sets the mint's
// page location; accountId selects the per-account identity (default identity
// when omitted). On CDP/browser failure the browser is recycled once and the
// mint retried once, so a crashed browser self-heals without a service restart.
export async function mintOnceCdp(editorUrl, accountId) {
  try {
    return await mintOnceOnWarmBrowser(editorUrl, accountId);
  } catch (e) {
    console.error('mint failed, recycling browser:', e && e.message);
    await closeBrowser();
    return await mintOnceOnWarmBrowser(editorUrl, accountId);
  }
}

async function mintOnceOnWarmBrowser(editorUrl, accountId) {
  const b = await ensureBrowser();
  const identity = identityForAccount(accountId);
  const s = await openMintSession(b, identity);
  try {
    const minted = await Promise.race([
      (async () => {
        await applyIdentity(b, s, identity);
        const { token, replayed } = await runMint(b, s, editorUrl);
        return { token, collectedLength: token.length, replayed, identity: identity.name };
      })(),
      new Promise((_, rej) => setTimeout(() => rej(new Error('mint-timeout')), MINT_TIMEOUT_MS)),
    ]);
    return minted;
  } finally {
    await closeMintSession(b, s);
  }
}

// Clean shutdown hook for the service: kill the warm browser and its profile.
export async function closeCdpBrowser() {
  await closeBrowser();
}

// Whether the warm browser is up (service ping reports this).
export function browserWarm() {
  return !!(browser && browser.ws.readyState === 1);
}
