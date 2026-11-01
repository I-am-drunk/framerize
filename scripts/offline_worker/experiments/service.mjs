// The actual system: a persistent token service.
// One small Node process answers mint requests over a Unix socket.
// Each request builds a fresh realm (about 250 ms, no browser, no JSDOM),
// so every token is freshly minted and nothing goes stale between requests.
import net from 'node:net';
import fs from 'node:fs';
import { execFile } from 'node:child_process';
import * as col from '../collection.mjs';
import * as pins from '../pins.mjs';
import vm from 'node:vm';
import { ScriptModules } from '../script_loader.mjs';
import { buildRealmGlobals } from './realm.mjs';

const SOCK = process.env.FRAMERIZE_TOKEN_SOCK
  || '/home/tjninja/.local/share/framerize/token.sock';
const MINT_BUDGET_MS = 8000;
const MINT_TIMEOUT_MS = 9000;

const startedAt = Date.now();
// The realm's seeded device id, so callers can align their fire page storage.
let REALM_CUID = null;
try {
  const g = buildRealmGlobals({ publicKey: pins.PUBLIC_KEY });
  REALM_CUID = (g.window.localStorage.getItem('__cuid')) || null;
} catch {}
let mintCount = 0;
let failCount = 0;
let busy = false;

// Real interaction trace, captured once in the user's own tab through the
// devtools Input domain (genuine trusted events, real timing). Replayed
// through the realm's registered listeners before each mint so the token's
// behavioral content is not silent. If absent, mint still works but stays
// behaviorally silent, which the live evidence shows Castle rejects.
let TRACE = [];
try {
  const t = JSON.parse(fs.readFileSync(
    new URL('./interaction-trace.json', import.meta.url), 'utf8'));
  if (Array.isArray(t.events)) TRACE = t.events;
} catch {}

// Replay the captured trace through the SDK's registered listeners.
// Dispatches every event through the realm hook; handlers run synchronously.
// Returns how many handler invocations fired.
// Replay the captured trace with its genuine inter-event timing. Each event's
// captured dt (ms offset from trace start) becomes a monotonic timeStamp, and
// the loop sleeps the real gaps so the SDK observes the true rhythm, not a
// burst. isTrusted cannot be made true from JS; that is a known, named limit.
// Randomize the interaction per mint: replaying the identical path every
// time is itself a bot signature. Each mint picks a fresh random rotation,
// translation, and playback rate so the path looks like a fresh human session.
// Every event still comes from the user's real captured trace; we only move
// and time it the way a real person would differ run to run. Nothing invented.
function jitterTrace() {
  const cx = 390, cy = 410;              // trace centroid (x 218-566, y 178-658)
  const ang = Math.random() * Math.PI * 2;
  const cos = Math.cos(ang), sin = Math.sin(ang);
  const dx = (Math.random() - 0.5) * 240; // up to +/-120 px translation
  const dy = (Math.random() - 0.5) * 200;
  const rate = 0.85 + Math.random() * 0.5; // playback 0.85x-1.35x
  return TRACE.map(ev => {
    const e = { ...ev };
    if (typeof e.clientX === 'number' && typeof e.clientY === 'number') {
      const rx = (e.clientX - cx) * cos - (e.clientY - cy) * sin;
      const ry = (e.clientX - cx) * sin + (e.clientY - cy) * cos;
      e.clientX = Math.round(cx + rx + dx);
      e.clientY = Math.round(cy + ry + dy);
    }
    if (typeof e.dt === 'number') e.dt = e.dt / rate;
    return e;
  });
}
async function replayTrace(win) {
  if (!win || typeof win.__dispatchCaptured !== 'function' || TRACE.length === 0) return 0;
  let fired = 0;
  const base = win.performance ? win.performance.now() : Date.now();
  const trace = jitterTrace();
  let prevDt = 0;
  for (const ev of trace) {
    const { type, dt, ...props } = ev;
    const gap = (typeof dt === 'number' ? dt : prevDt) - prevDt;
    if (gap > 0) await new Promise(r => setTimeout(r, Math.min(gap, 1000)));
    if (typeof dt === 'number') prevDt = dt;
    try { fired += win.__dispatchCaptured(type, { ...props, timeStamp: base + (typeof dt === 'number' ? dt : 0) }) || 0; } catch {}
  }
  return fired;
}

// Mint one token in a fresh realm. Mirrors experiments/script_mint.mjs,
// which is the proven path (765-char token, collected: true).
const ENCODE_TOKEN = new URL('../../../target/debug/examples/encode-token', import.meta.url).pathname;

// Run encode-token on an FCLT frame; resolve with the wire token string.
function encodeWireToken(fcltFrame) {
  return new Promise((resolve, reject) => {
    execFile(ENCODE_TOKEN, { maxBuffer: 4 * 1024 * 1024 }, (err, stdout, stderr) => {
      if (err) return reject(new Error('encode-token: ' + (stderr || err.message)));
      const token = String(stdout).trim();
      if (!token) return reject(new Error('encode-token: empty'));
      resolve(token);
    }).stdin.end(fcltFrame);
  });
}

// Mint one wire token in a fresh realm via the production path:
// vm.createContext(buildRealmGlobals) -> ScriptModules.mint (collection JSON)
// -> FCLT frame -> encode-token -> compact wire token.
async function mintOnce(reqCuid) {
  const globals = buildRealmGlobals({ publicKey: pins.PUBLIC_KEY, seedCuid: reqCuid || REALM_CUID || undefined });
  const context = vm.createContext(globals);
  try {
    const modules = new ScriptModules(context);
    await modules.prepare();
    await modules.evaluate();
    // Listeners are registered now. Replay the real interaction trace so the
    // token carries genuine behavioral content instead of silence.
    const replayed = await replayTrace(globals);
    const collection = await Promise.race([
      modules.mint(),
      new Promise((_, rej) => setTimeout(() => rej(new Error('mint-timeout')), MINT_TIMEOUT_MS)),
    ]);
    if (typeof collection !== 'string' || collection.length === 0)
      throw new Error('collection-shape: ' + typeof collection);
    // Serialize the collection JSON to an FCLT frame, then encode.
    const body = Buffer.from(collection, 'utf8');
    const header = Buffer.alloc(12);
    header.write('FCLT'); header[4] = 1; header.writeUInt32BE(body.length, 8);
    const frame = Buffer.concat([header, body]);
    const token = await encodeWireToken(frame);
    return { token, collectedLength: collection.length, replayed };
  } finally {}
}

async function handle(req) {
  if (req.op === 'ping') {
    const res = { ok: true, uptime_ms: Date.now() - startedAt,
      mints: mintCount, failures: failCount, busy,
      rss_mb: Math.round(process.memoryUsage().rss / 1048576) };
    if (process.env.FRAMERIZE_MINT_BACKEND === 'cdp') {
      try {
        const { browserWarm } = await import('./service_cdp.mjs');
        res.cdp_browser_warm = browserWarm();
      } catch {}
    }
    return res;
  }
  if (req.op !== 'mint') return { ok: false, error: 'unknown-op' };
  if (busy) return { ok: false, error: 'busy' };
  busy = true;
  try {
    const reqCuid = (typeof req.cuid === 'string' && /^[A-Za-z0-9._~-]{1,256}$/.test(req.cuid)) ? req.cuid : null;
    // Backend switch: FRAMERIZE_MINT_BACKEND=cdp mints via one headless Chrome
    // page (~200 MB, genuine engine answers); anything else uses the realm.
    // editorUrl threads the project's editor page to the CDP mint (the passing
    // browser path mints there), defaulting to the /projects index.
    let minted;
    if (process.env.FRAMERIZE_MINT_BACKEND === 'cdp') {
      const { mintOnceCdp } = await import('./service_cdp.mjs');
      const editorUrl = (typeof req.editor_url === 'string' && req.editor_url.startsWith('https://framer.com/projects/'))
        ? req.editor_url : 'https://framer.com/projects';
      // account_id selects a stable per-account browser identity (task 3).
      const accountId = (typeof req.account_id === 'string' && /^[A-Za-z0-9._~-]{1,256}$/.test(req.account_id)) ? req.account_id : null;
      minted = await mintOnceCdp(editorUrl, accountId);
    } else {
      minted = await mintOnce(reqCuid);
    }
    const { token, collectedLength, replayed, identity } = minted;
    mintCount++;
    // token is the compact wire string (clientId|payload). Send it directly.
    return { ok: true, token, cuid: reqCuid || REALM_CUID, token_length: token.length,
      collected_length: collectedLength, replayed, identity,
      trace_events: TRACE.length, encoding: 'wire-token' };
  } catch (e) {
    failCount++;
    return { ok: false, error: String(e && e.message || e) };
  } finally {
    busy = false;
  }
}

const server = net.createServer(conn => {
  let buf = '';
  conn.on('data', chunk => {
    buf += chunk;
    let i;
    while ((i = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, i).trim();
      buf = buf.slice(i + 1);
      if (!line) continue;
      let req;
      try { req = JSON.parse(line); }
      catch { conn.write(JSON.stringify({ ok: false, error: 'bad-json' }) + '\n'); continue; }
      handle(req).then(res => conn.write(JSON.stringify(res) + '\n'));
    }
  });
  conn.on('error', () => {});
});

function shutdown() {
  const done = () => {
    server.close(() => process.exit(0));
    try { fs.unlinkSync(SOCK); } catch {}
    setTimeout(() => process.exit(0), 1000).unref();
  };
  if (process.env.FRAMERIZE_MINT_BACKEND === 'cdp') {
    import('./service_cdp.mjs').then(m => m.closeCdpBrowser()).then(done, done);
  } else {
    done();
  }
}
process.on('SIGTERM', shutdown);
process.on('SIGINT', shutdown);

try { fs.unlinkSync(SOCK); } catch {}
fs.mkdirSync(SOCK.slice(0, SOCK.lastIndexOf('/')), { recursive: true });
server.listen(SOCK, () => {
  fs.chmodSync(SOCK, 0o600);
  console.log('token service listening on ' + SOCK);
});
