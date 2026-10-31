// Assemble the single authorized live request for the no-browser path.
// Mints a fresh token from the service, builds the exact request shape the
// passing browser path uses, and writes the request file the Go sender reads.
// Does NOT fire. The Go sender fires once when run with LIVE_REQUEST_FILE.
// usage: node prepare_live.mjs <projectId> <bearerToken> <cookieHeader>
import fs from 'node:fs';
import net from 'node:net';
const [projectId, bearer, cookie] = process.argv.slice(2);
if (!projectId || !bearer || !cookie) {
  console.error('usage: prepare_live.mjs <projectId> <bearerToken> <cookieHeader>');
  process.exit(2);
}
function mint() {
  return new Promise((resolve, reject) => {
    const conn = net.createConnection('/home/tjninja/.local/share/framerize/token.sock');
    let buf = '';
    const t = setTimeout(() => { conn.destroy(); reject(new Error('mint-timeout')); }, 15000);
    conn.on('connect', () => conn.write(JSON.stringify({ op: 'mint' }) + '\n'));
    conn.on('data', c => { buf += c; const i = buf.indexOf('\n');
      if (i >= 0) { clearTimeout(t); conn.end(); try { resolve(JSON.parse(buf.slice(0, i))); } catch (e) { reject(e); } } });
    conn.on('error', reject);
  });
}
const reply = await mint();
if (!reply.ok) { console.error('mint failed: ' + reply.error); process.exit(1); }
const token = reply.token;
// Match the passing browser path's envelope exactly (crates/framer-api/src/
// proxy.rs): model at top level, the client payload nested under request.
const body = JSON.stringify({
  projectId, featureCategory: 'agents', featureName: 'agents',
  model: 'google/gemini-2.5-flash', step: 'message',
  sessionId: 'nb-' + Math.random().toString(16).slice(2, 8), stepIndex: 0,
  turnId: Array.from({ length: 32 }, () => Math.floor(Math.random() * 16).toString(16)).join(''),
  userPrompt: 'canary', castleToken: token,
  request: { model: 'google/gemini-2.5-flash', messages: [{ role: 'user', content: 'ok' }], max_tokens: 10, stream: false },
});
const spec = {
  addr: 'api.framer.com:443',
  host: 'api.framer.com',
  path: '/ai/v3/messages/',
  headers: {
    // The passing browser path fires this fetch from the editor page on
    // https://framer.com, so Chrome adds these automatically. Castle scores
    // the token against the mint environment (same TLS, same UA, same Origin);
    // the realm mints with location https://framer.com/projects, so Origin and
    // Referer must match that or the request reads as a different environment.
    'accept': '*/*',
    'accept-encoding': 'gzip, deflate, br, zstd',
    'accept-language': 'en-US,en;q=0.9',
    'content-type': 'application/json; charset=utf-8',
    'origin': 'https://framer.com',
    // Referer must be the project's editor page (where the token was minted),
    // matching editor residency, not the site root.
    'referer': 'https://framer.com/projects/' + projectId,
    'sec-ch-ua': '"Chromium";v="155", "Google Chrome";v="155", "Not_A Brand";v="99"',
    'sec-ch-ua-mobile': '?0',
    'sec-ch-ua-platform': '"Linux"',
    'sec-fetch-dest': 'empty',
    'sec-fetch-mode': 'cors',
    'sec-fetch-site': 'same-site',
    'user-agent': 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36',
    'x-requested-by': 'Framer',
    'authorization': 'Bearer ' + bearer,
    'cookie': cookie,
  },
  body,
};
const out = new URL('./live_request.json', import.meta.url).pathname;
fs.writeFileSync(out, JSON.stringify(spec, null, 1));
console.log(JSON.stringify({ wrote: out, tokenLength: token.length, bodyBytes: body.length }));
