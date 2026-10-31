// Step 4: ONE live Framer request with a service-minted Castle token.
// Fired inside a real framer.com tab (real browser TLS + session), so the
// only thing that differs from a normal Framer request is where the token
// was minted. Records the status; does NOT retry. Usage:
//   node step4_fire.mjs <framerTabId> <projectId> [token_b64_file]
// Default mints fresh from the token service (tokens age; a stale staged
// file wastes the shot). Pass a file only to replay a specific token.
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
const [tabId, projectId, tokenB64] = process.argv.slice(2);
if (!tabId || !projectId) { console.error('usage: step4_fire.mjs <tabId> <projectId> [token_b64_file]'); process.exit(2); }
async function mintFresh() {
  const SOCK = process.env.FRAMERIZE_TOKEN_SOCK
    || '/home/tjninja/.local/share/framerize/token.sock';
  const reply = await new Promise((resolve, reject) => {
    const conn = net.createConnection(SOCK);
    let buf = '';
    const t = setTimeout(() => { conn.destroy(); reject(new Error('mint-timeout')); }, 15000);
    conn.on('connect', () => conn.write(JSON.stringify({ op: 'mint' }) + '\n'));
    conn.on('data', c => { buf += c; const i = buf.indexOf('\n');
      if (i >= 0) { clearTimeout(t); conn.end(); resolve(JSON.parse(buf.slice(0, i))); } });
    conn.on('error', reject);
  });
  if (!reply.ok) throw new Error('mint failed: ' + reply.error);
  if (reply.token) return reply.token;  // wire token (current service)
  return JSON.parse(Buffer.from(reply.token_b64, 'base64').toString('utf8'));  // legacy
}
const token = tokenB64
  ? (fs.readFileSync(tokenB64, 'utf8').trim().startsWith('c') ? fs.readFileSync(tokenB64, 'utf8').trim()
     : JSON.parse(Buffer.from(fs.readFileSync(tokenB64, 'utf8').trim(), 'base64').toString('utf8')))
  : await mintFresh();
function get(path){return new Promise((res,rej)=>{http.get({host:'127.0.0.1',port:9222,path},r=>{let b='';r.on('data',c=>b+=c);r.on('end',()=>res(JSON.parse(b)));}).on('error',rej);});}
const tab=(await get('/json/list')).find(t=>t.id===tabId);
if (!tab) throw new Error('tab not found');
const { default: WebSocket } = await import('/home/tjninja/framerize-browserless/scripts/offline_worker/inputs/dom/node_modules/ws/wrapper.mjs');
const ws=new WebSocket(tab.webSocketDebuggerUrl,{perMessageDeflate:false});
await new Promise((res,rej)=>{ws.on('open',res);ws.on('error',rej);});
let msgId = 0; const pending = new Map();
ws.on('message', d => { const m = JSON.parse(d); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } });
function cdp(method, params = {}) { const id = ++msgId; return new Promise((res, rej) => { pending.set(id, res); ws.send(JSON.stringify({ id, method, params })); setTimeout(() => rej(new Error('cdp timeout ' + method)), 30000); }); }

// 1. bearer (session-gated, not an AI call, does not spend the shot)
const bearerRes = await cdp('Runtime.evaluate', { expression: `(async()=>{const r=await fetch('https://api.framer.com/auth/ai/access-token',{credentials:'include'});const j=await r.json();return j.accessToken||j.token||null;})()`, returnByValue: true, awaitPromise: true });
const bearer = bearerRes.result && bearerRes.result.result && bearerRes.result.result.value;
if (!bearer) { console.error(JSON.stringify({ status: 'no-bearer', detail: 'session not available in tab' })); process.exit(1); }

// 2. THE one live request. Fires exactly once. Records the raw status.
const body = {
  projectId, featureCategory: 'agents', featureName: 'agents', step: 'message',
  sessionId: 'step4-' + Math.random().toString(16).slice(2, 8), stepIndex: 0,
  turnId: Array.from({length:32},()=>Math.floor(Math.random()*16).toString(16)).join(''),
  userPrompt: 'canary', castleToken: token,
  request: { model: 'google/gemini-2.5-flash', messages: [{ role: 'user', content: 'ok' }], max_tokens: 10, stream: false },
};
const fire = await cdp('Runtime.evaluate', { expression: `(async([b,body])=>{
  const r = await fetch('https://api.framer.com/ai/v3/messages/', { method:'POST', credentials:'include',
    headers:{ 'content-type':'application/json; charset=utf-8','x-requested-by':'Framer','authorization':'Bearer '+b },
    body: JSON.stringify(body) });
  const text = await r.text();
  return { status: r.status, bodyHead: text.slice(0,300) };
})(${JSON.stringify([bearer, body])})`, returnByValue: true, awaitPromise: true });
const out = fire.result && fire.result.result && fire.result.result.value;
console.log(JSON.stringify({ fired: true, result: out }));
ws.close();
