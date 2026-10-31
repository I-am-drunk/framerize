// Step 4, pure-Node path: ONE live Framer request with a service-minted
// Castle token, sent over Node's own HTTPS stack (no browser). This is the
// goal's actual end state. If it 403s, the remaining suspect is Node's TLS
// fingerprint, exactly as the goal's step 4 names. Fires once, no retry.
// usage: node step4_fire_node.mjs <sessionCookieFile> <projectId> <token_b64_file> [bearerFile]
import fs from 'node:fs';
const [sessFile, projectId, tokenB64File, bearerFile] = process.argv.slice(2);
if (!sessFile || !projectId || !tokenB64File) { console.error('usage: step4_fire_node.mjs <sessionCookieFile> <projectId> <token_b64_file> [bearerFile]'); process.exit(2); }
const cookieRaw = fs.readFileSync(sessFile, 'utf8').trim();
let token = fs.readFileSync(tokenB64File, 'utf8').trim();
// Backward compat: if given a base64 JSON array, reject with guidance.
if (!token.startsWith('c') && token.length < 100) {
  try { const arr = JSON.parse(Buffer.from(token, 'base64').toString('utf8'));
    if (Array.isArray(arr)) { console.error('need wire token STRING, got collection array'); process.exit(2); } } catch {}
}
async function post(path, body, bearer) {
  const headers = { 'content-type': 'application/json; charset=utf-8',
    'x-requested-by': 'Framer', 'cookie': cookieRaw };
  if (bearer) headers['authorization'] = 'Bearer ' + bearer;
  const r = await fetch('https://api.framer.com' + path, {
    method: 'POST', headers, body: JSON.stringify(body) });
  return { status: r.status, bodyHead: (await r.text()).slice(0, 300) };
}
async function getBearer() {
  if (bearerFile && fs.existsSync(bearerFile)) return fs.readFileSync(bearerFile, 'utf8').trim();
  const r = await fetch('https://api.framer.com/auth/ai/access-token', { headers: { 'cookie': cookieRaw } });
  const j = await r.json();
  return j.accessToken || j.token || null;
}
const bearer = await getBearer();
if (!bearer) { console.log(JSON.stringify({ fired: false, reason: 'no-bearer' })); process.exit(1); }
const body = {
  projectId, featureCategory: 'agents', featureName: 'agents', step: 'message',
  sessionId: 'step4node-' + Math.random().toString(16).slice(2, 8), stepIndex: 0,
  turnId: Array.from({length:32},()=>Math.floor(Math.random()*16).toString(16)).join(''),
  userPrompt: 'canary', castleToken: token,
  request: { model: 'google/gemini-2.5-flash', messages: [{ role: 'user', content: 'ok' }], max_tokens: 10, stream: false },
};
const res = await post('/ai/v3/messages/', body, bearer);
console.log(JSON.stringify({ fired: true, path: '/ai/v3/messages/', via: 'pure-node', result: res }));
