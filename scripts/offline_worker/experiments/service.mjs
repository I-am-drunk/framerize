// The actual system: a persistent token service.
// One small Node process answers mint requests over a Unix socket.
// Each request builds a fresh realm (about 250 ms, no browser, no JSDOM),
// so every token is freshly minted and nothing goes stale between requests.
import net from 'node:net';
import fs from 'node:fs';
import { execFile } from 'node:child_process';
import * as col from '../collection.mjs';
import * as pins from '../pins.mjs';
import { createRealmWindow } from '../realm_window.mjs';
import { SdkModules } from '../sdk_loader.mjs';
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
// createRealmWindow (workers installed) -> SdkModules.mint (collection JSON)
// -> FCLT frame -> encode-token -> compact wire token.
async function mintOnce(reqCuid) {
  const dom = createRealmWindow(pins.PUBLIC_KEY, reqCuid || REALM_CUID || undefined);
  dom.window.castlePublishableAPIKey = pins.PUBLIC_KEY;
  if (dom.window.__workerInstallError) throw new Error('workers: ' + dom.window.__workerInstallError);
  try {
    const modules = new SdkModules(dom.getInternalVMContext());
    await modules.prepare();
    await modules.evaluate();
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
    return { token, collectedLength: collection.length };
  } finally {
    try { await dom.close(); } catch {}
  }
}

async function handle(req) {
  if (req.op === 'ping') {
    return { ok: true, uptime_ms: Date.now() - startedAt,
      mints: mintCount, failures: failCount, busy,
      rss_mb: Math.round(process.memoryUsage().rss / 1048576) };
  }
  if (req.op !== 'mint') return { ok: false, error: 'unknown-op' };
  if (busy) return { ok: false, error: 'busy' };
  busy = true;
  try {
    const reqCuid = (typeof req.cuid === 'string' && /^[A-Za-z0-9._~-]{1,256}$/.test(req.cuid)) ? req.cuid : null;
    const { token, collectedLength } = await mintOnce(reqCuid);
    mintCount++;
    // token is the compact wire string (clientId|payload). Send it directly.
    return { ok: true, token, cuid: reqCuid || REALM_CUID, token_length: token.length,
      collected_length: collectedLength, encoding: 'wire-token' };
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
  server.close(() => process.exit(0));
  try { fs.unlinkSync(SOCK); } catch {}
  setTimeout(() => process.exit(0), 1000).unref();
}
process.on('SIGTERM', shutdown);
process.on('SIGINT', shutdown);

try { fs.unlinkSync(SOCK); } catch {}
fs.mkdirSync(SOCK.slice(0, SOCK.lastIndexOf('/')), { recursive: true });
server.listen(SOCK, () => {
  fs.chmodSync(SOCK, 0o600);
  console.log('token service listening on ' + SOCK);
});
