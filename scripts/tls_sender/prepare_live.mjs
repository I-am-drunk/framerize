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
const body = JSON.stringify({
  projectId, featureCategory: 'agents', featureName: 'agents', step: 'message',
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
    'content-type': 'application/json; charset=utf-8',
    'x-requested-by': 'Framer',
    'authorization': 'Bearer ' + bearer,
    'cookie': cookie,
  },
  body,
};
const out = new URL('./live_request.json', import.meta.url).pathname;
fs.writeFileSync(out, JSON.stringify(spec, null, 1));
console.log(JSON.stringify({ wrote: out, tokenLength: token.length, bodyBytes: body.length }));
