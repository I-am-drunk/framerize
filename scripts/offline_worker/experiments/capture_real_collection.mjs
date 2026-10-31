// Capture the real browser's own 765-slot collection, read-only and
// no-network, in a blank tab. The realm's graph bundle runs with __collect
// hooked; real Chrome answers every question, __collect captures the final
// array. Nothing is sent anywhere.
import fs from 'node:fs';
import http from 'node:http';
const tabId = process.argv[2];
if (!tabId) { console.error('usage: capture_real_collection.mjs <tabId>'); process.exit(2); }
const bundle = fs.readFileSync('/tmp/graph_bundle_browser.js', 'utf8');
const OUT = '/tmp/real-collection.json';
function get(path) {
  return new Promise((res, rej) => {
    http.get({ host: '127.0.0.1', port: 9222, path }, r => {
      let b=''; r.on('data',c=>b+=c); r.on('end',()=>res(JSON.parse(b)));
    }).on('error', rej);
  });
}
const targets = await get('/json/list');
const tab = targets.find(t => t.id === tabId);
if (!tab) throw new Error('tab not found');
const { default: WebSocket } = await import('/home/tjninja/framerize-browserless/scripts/offline_worker/inputs/dom/node_modules/ws/wrapper.mjs');
const ws = new WebSocket(tab.webSocketDebuggerUrl, { perMessageDeflate: false });
await new Promise((res, rej) => { ws.on('open', res); ws.on('error', rej); });
const runner = `(async () => {
  let collected = null;
  window.__collect = v => { collected = v; return v; };
  window.__sharedWorker = v => v;
  window.__network = v => v;
  try {
    (0, eval)(${JSON.stringify(bundle)});
    const wrapper = window.__modules.wrapper;
    const mint = wrapper.a;
    const token = await Promise.race([
      mint({ budget_ms: 8000 }),
      new Promise((_, rej) => setTimeout(() => rej(new Error('mint-timeout')), 9000)),
    ]);
    return { ok: true, slots: collected ? collected.length : 0, collected };
  } catch (e) {
    return { ok: false, error: String(e && e.message || e) };
  }
})()`;
const reply = await new Promise((resolve, reject) => {
  const id = 7;
  ws.on('message', d => { const m = JSON.parse(d); if (m.id === id) resolve(m); });
  ws.on('error', reject);
  ws.send(JSON.stringify({ id, method: 'Runtime.evaluate',
    params: { expression: runner, returnByValue: true, awaitPromise: true, timeout: 20000 } }));
});
const val = reply.result && reply.result.result && reply.result.result.value;
if (!val) { console.error('no value', JSON.stringify(reply).slice(0, 400)); process.exit(1); }
if (!val.ok) { console.error('RUN ERR', val.error); process.exit(1); }
fs.writeFileSync(OUT, JSON.stringify(val.collected));
console.log('ok slots=', val.slots, '->', OUT);
ws.close();
