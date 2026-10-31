// Minimal CDP mint engine: one headless Chrome page, real engine, trusted
// events via the Input domain, Castle SDK mints a genuine token.
// RAM target: one page, no UI, no extra tabs. Measured after mint.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import * as pins from '../offline_worker/pins.mjs';

const CHROME = '/home/tjninja/.local/share/lets-study-test-browser/chrome/opt/google/chrome/chrome';
// Unique per run so two mints never collide and the live service is untouched.
const RUN = process.env.CDP_MINT_RUN || String(process.pid);
const PORT = 9333 + (parseInt(RUN, 36) % 500);
// Editor residency: the caller passes the project's editor URL
// (https://framer.com/projects/<id>) as argv[2]. The mint must present that
// location so Castle sees the same context as the passing browser path
// (crates/framer-api proxy.rs mints on the editor page). Optional: about:blank
// is the offline default.
const PROFILE = '/tmp/cdp-mint-profile-' + RUN;
const TRACE = JSON.parse(fs.readFileSync(new URL('../offline_worker/experiments/interaction-trace.json', import.meta.url), 'utf8')).events;

function get(path) {
  return new Promise((res, rej) => {
    http.get({ host: '127.0.0.1', port: PORT, path }, r => {
      let b = ''; r.on('data', c => b += c); r.on('end', () => { try { res(JSON.parse(b)); } catch (e) { rej(e); } });
    }).on('error', rej);
  });
}
const sleep = ms => new Promise(r => setTimeout(r, ms));

async function waitReady() {
  for (let i = 0; i < 60; i++) {
    try { const v = await get('/json/version'); if (v && v.Browser) return; } catch {}
    await sleep(250);
  }
  throw new Error('chrome devtools not ready');
}

function launch() {
  const args = [
    '--headless=new', '--no-first-run', '--no-default-browser-check',
    '--disable-extensions', '--disable-sync', '--disable-background-networking',
    '--disable-component-update', '--mute-audio', '--no-sandbox',
    '--window-size=1280,800', '--force-device-scale-factor=1',
    '--disable-gpu', '--disable-software-rasterizer', '--disable-dev-shm-usage',
    '--disable-features=Translate,BackForwardCache,AcceptCHFrame,MediaRouter,OptimizationHints,InterestFeedContentSuggestions,CalculateNativeWinOcclusion',
    '--disable-breakpad', '--disable-crash-reporter', '--disable-logging', '--no-zygote',
    '--disable-renderer-backgrounding', '--disable-background-timer-throttling',
    '--js-flags=--max-old-space-size=128',
    '--remote-debugging-port=' + PORT,
    '--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36',
    '--user-data-dir=' + PROFILE,
    'about:blank',  // explicit Page.navigate happens before graph load
  ];
  const p = spawn(CHROME, args, { stdio: ['ignore', 'ignore', 'pipe'] });
  p.stderr.on('data', () => {});
  return p;
}

// Load the Castle SDK modules into the real page. We reuse the pinned chunk
// bytes and link them through a small in-page module loader via <script type=module>.
async function main() {
  const chrome = launch();
  const cleanup = () => { try { chrome.kill('SIGKILL'); } catch {} };
  process.on('exit', cleanup);
  await waitReady();
  const targets = await get('/json/list');
  const page = targets.find(t => t.type === 'page');
  if (!page) throw new Error('no page target');
  const { default: WebSocket } = await import('../offline_worker/inputs/dom/node_modules/ws/wrapper.mjs');
  const ws = new WebSocket(page.webSocketDebuggerUrl, { perMessageDeflate: false, maxPayload: 64 * 1024 * 1024 });
  await new Promise((res, rej) => { ws.on('open', res); ws.on('error', rej); });
  let mid = 0;
  const pending = new Map();
  ws.on('message', d => {
    let m; try { m = JSON.parse(d); } catch { return; }
    if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
  });
  const send = (method, params) => new Promise((res, rej) => {
    const id = ++mid; pending.set(id, m => m.error ? rej(new Error(method + ': ' + JSON.stringify(m.error))) : res(m.result));
    ws.send(JSON.stringify({ id, method, params }));
  });
  await send('Page.enable', {});
  await send('Runtime.enable', {});
  console.log('chrome pid', chrome.pid, 'page', page.id);
  globalThis.__send = send; globalThis.__ws = ws; globalThis.__chrome = chrome;
  return { send, ws, chrome };
}

// Build the graph script with NO collection transform so the wrapper's mint
// returns the genuine wire token (createRequestToken output) from the real
// engine. The realm used the transform only for offline slot analysis.
async function buildBrowserBundle() {
  const { buildGraphScript } = await import('../offline_worker/experiments/graph_script.mjs');
  return buildGraphScript({
    assets: pins.ASSETS, modules: pins.MODULES, cdn: pins.CDN,
    collectionTransform: null,
  });
}

async function evaluate(send, expr, timeout = 30000) {
  const r = await send('Runtime.evaluate', {
    expression: expr, returnByValue: true, awaitPromise: true, timeout,
  });
  if (r.exceptionDetails) throw new Error('eval: ' + JSON.stringify(r.exceptionDetails).slice(0, 400));
  return r.result ? r.result.value : undefined;
}

async function run() {
  const { send, ws, chrome } = await main();
  const t0 = Date.now();
  // 1. Load the SDK graph into the real page (real Chrome answers everything).
  // Navigate to the framer.com index so the mint runs on the framer.com
  // origin (the passing browser path mints on framer.com). Wait for load.
  const loaded = new Promise(res => {
    const h = d => { try { const m = JSON.parse(d); if (m.method === 'Page.loadEventFired') { ws.off('message', h); res(); } } catch {} };
    ws.on('message', h); setTimeout(res, 15000);
  });
  await send('Page.navigate', { url: 'https://framer.com/projects' });
  await loaded;
  console.log('navigated, location:', await evaluate(send, 'location.href'));
  const bundle = await buildBrowserBundle();
  await evaluate(send, `(0,eval)(${JSON.stringify(bundle)}); 'graph-loaded'`);
  // 2. Set the publishable key and configure.
  await evaluate(send, `window.castlePublishableAPIKey = ${JSON.stringify(pins.PUBLIC_KEY)}; 'key-set'`);
  console.log('graph loaded in', Date.now() - t0, 'ms');
  console.log('mint location:', await evaluate(send, 'location.href'));
  // Editor residency: set the page location to the project's editor URL so
  // Castle's document.location reads match the passing browser path. No signal
  // is invented; only the URL bar changes, every other answer stays Chrome's.
  const editorUrl = process.argv[2];
  if (editorUrl && editorUrl.startsWith('https://framer.com/projects/')) {
    await evaluate(send, `history.replaceState(null, '', ${JSON.stringify(editorUrl)}); location.pathname`);
    console.log('editor url set:', editorUrl);
  } else {
    console.log('editor url: about:blank (offline default)');
  }
  // 3. Replay the real interaction trace via the Input domain (trusted events).
  const base = Date.now();
  let prevDt = 0;
  for (const ev of TRACE) {
    const gap = (typeof ev.dt === 'number' ? ev.dt : prevDt) - prevDt;
    if (gap > 0) await sleep(Math.min(gap, 1000));
    if (typeof ev.dt === 'number') prevDt = ev.dt;
    await dispatchInput(send, ev);
  }
  console.log('trace replayed');
  // 4. Mint a genuine wire token from the real engine.
  const token = await evaluate(send, `(async () => {
    const m = globalThis.__modules.wrapper; if (!m || !m.a) return { err: 'no wrapper' };
    try { const tok = await m.a({ budget_ms: 8000 }); return { tok }; }
    catch (e) { return { err: String(e && e.message || e) }; }
  })()`, 15000);
  console.log('mint result:', token && token.tok ? 'token len ' + token.tok.length : JSON.stringify(token));
  const rss = treeRss(chrome.pid);
  console.log('chrome-tree-rss-mb', rss);
  const outPath = process.env.CDP_MINT_TOKEN_OUT || ('/tmp/cdp-wire-token-' + RUN + '.txt');
  if (token && token.tok) { fs.writeFileSync(outPath, token.tok, { mode: 0o600 }); console.log('token written:', outPath); }
  // 5. Report RSS of the chrome process tree.
  chrome.kill('SIGKILL');
}

// Map a captured trace event to a CDP Input-domain dispatch, which the engine
// treats as trusted (isTrusted=true), unlike JS dispatchEvent.
async function dispatchInput(send, ev) {
  const t = ev.type;
  try {
    if (t === 'mousemove' || t === 'pointermove') {
      await send('Input.dispatchMouseEvent', {
        type: 'mouseMoved', x: ev.clientX ?? ev.x ?? 0, y: ev.clientY ?? ev.y ?? 0,
        pointerType: 'mouse',
      });
    } else if (t === 'mousedown') {
      await send('Input.dispatchMouseEvent', { type: 'mousePressed', x: ev.clientX ?? ev.x ?? 0, y: ev.clientY ?? ev.y ?? 0, button: 'left', clickCount: 1 });
    } else if (t === 'mouseup') {
      await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: ev.clientX ?? ev.x ?? 0, y: ev.clientY ?? ev.y ?? 0, button: 'left', clickCount: 1 });
    } else if (t === 'keydown' || t === 'keypress' || t === 'keyup') {
      await send('Input.dispatchKeyEvent', {
        type: t === 'keyup' ? 'keyUp' : 'rawKeyDown',
        key: ev.key ?? '', code: ev.code ?? '', text: t === 'keypress' ? (ev.key ?? '') : undefined,
      });
    } else if (t === 'wheel') {
      await send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: ev.clientX ?? ev.x ?? 0, y: ev.clientY ?? ev.y ?? 0, deltaX: ev.deltaX ?? 0, deltaY: ev.deltaY ?? 0 });
    }
    // focus/visibility/touch are not reproducible via Input; skip silently.
  } catch {}
}

run().catch(e => { console.error('RUN ERR', e && e.message); process.exit(1); });

// Sum RSS of the chrome process tree in MB.
function treeRss(rootPid) {
  let total = 0;
  const seen = new Set();
  const queue = [rootPid];
  const childrenOf = pid => {
    try {
      return fs.readFileSync(`/proc/${pid}/task/${pid}/children`, 'utf8').trim().split(/\s+/).filter(Boolean).map(Number);
    } catch { return []; }
  };
  while (queue.length) {
    const pid = queue.shift();
    if (seen.has(pid)) continue;
    seen.add(pid);
    try {
      const status = fs.readFileSync(`/proc/${pid}/status`, 'utf8');
      const m = status.match(/VmRSS:\s+(\d+)\s+kB/);
      if (m) total += parseInt(m[1], 10);
    } catch {}
    for (const c of childrenOf(pid)) queue.push(c);
  }
  return Math.round(total / 1024);
}
