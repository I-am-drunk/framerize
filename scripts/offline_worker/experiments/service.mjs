// The actual system: a persistent token service.
// One small Node process answers mint requests over a Unix socket.
// Each request builds a fresh realm (about 250 ms, no browser, no JSDOM),
// so every token is freshly minted and nothing goes stale between requests.
import net from 'node:net';
import fs from 'node:fs';
import vm from 'node:vm';
import * as col from '../collection.mjs';
import * as pins from '../pins.mjs';
import * as realm from './realm.mjs';
import { buildGraphScript } from './graph_script.mjs';

const SOCK = process.env.FRAMERIZE_TOKEN_SOCK
  || '/home/tjninja/.local/share/framerize/token.sock';
const MINT_BUDGET_MS = 8000;
const MINT_TIMEOUT_MS = 9000;

const startedAt = Date.now();
let mintCount = 0;
let failCount = 0;
let busy = false;

// Build the SDK graph source once at startup; the bytes are pinned, so the
// script text is identical for every request. Only the realm context differs.
const graphScript = buildGraphScript({
  assets: pins.ASSETS, modules: pins.MODULES, cdn: pins.CDN,
  collectionTransform: col.collectionSource,
});

// Mint one token in a fresh realm. Mirrors experiments/script_mint.mjs,
// which is the proven path (765-char token, collected: true).
function mintOnce() {
  const globals = realm.buildRealmGlobals({ publicKey: pins.PUBLIC_KEY });
  const context = vm.createContext(globals);
  const script = graphScript
    .replace(/import\.meta\.collect\(/g, 'globalThis.__collect(')
    .replace(/import\.meta\.sharedWorker\(/g, 'globalThis.__sharedWorker(')
    .replace(/import\.meta\.network\(/g, 'globalThis.__network(');
  let collected = null;
  globals.__collect = v => { collected = v; return v; };
  globals.__sharedWorker = v => v;
  globals.__network = v => v;
  vm.runInContext(script, context, { filename: 'graph.bundle.js' });
  const wrapper = vm.runInContext('globalThis.__modules.wrapper', context);
  const mint = wrapper.a;
  return Promise.race([
    mint({ budget_ms: MINT_BUDGET_MS }),
    new Promise((_, rej) => setTimeout(() => rej(new Error('mint-timeout')), MINT_TIMEOUT_MS)),
  ]).then(token => ({ token, collectedLength: collected ? collected.length : 0 }));
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
    const { token, collectedLength } = await mintOnce();
    mintCount++;
    // Compact the 765-slot collection array for transport: the raw array is
    // tens of KB as JSON, so send base64 and let the caller expand it.
    const token_b64 = Buffer.from(JSON.stringify(token), 'utf8').toString('base64');
    return { ok: true, token_b64, token_length: token ? token.length : 0,
      collected_length: collectedLength, encoding: 'json-array-base64' };
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
