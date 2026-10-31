// Long real-interaction capture: drive the framer.com tab through the devtools
// Input domain for ~40s of sustained, varied, human-like activity while the
// shape recorder runs. Genuine trusted events with real timing; no key text,
// no page content. usage: node inject_trace_long.mjs <tabId>
import fs from 'node:fs';
import http from 'node:http';
const [tabId] = process.argv.slice(2);
if (!tabId) { console.error('usage: inject_trace_long.mjs <tabId>'); process.exit(2); }
function get(path){return new Promise((res,rej)=>{http.get({host:'127.0.0.1',port:9222,path},r=>{let b='';r.on('data',c=>b+=c);r.on('end',()=>res(JSON.parse(b)));}).on('error',rej);});}
const tab=(await get('/json/list')).find(t=>t.id===tabId);
if (!tab) throw new Error('tab not found');
const { default: WebSocket } = await import('/home/tjninja/framerize-browserless/scripts/offline_worker/inputs/dom/node_modules/ws/wrapper.mjs');
const ws=new WebSocket(tab.webSocketDebuggerUrl,{perMessageDeflate:false});
await new Promise((res,rej)=>{ws.on('open',res);ws.on('error',rej);});
let id=0; const pend=new Map();
ws.on('message',d=>{const m=JSON.parse(d);if(m.id&&pend.has(m.id)){pend.get(m.id)(m);pend.delete(m.id);}});
function cdp(method,params={}){const i=++id;return new Promise((res,rej)=>{pend.set(i,res);ws.send(JSON.stringify({id:i,method,params}));setTimeout(()=>rej(new Error('timeout '+method)),15000);});}
const sleep = ms => new Promise(r => setTimeout(r, ms));
const rand = (a, b) => a + Math.random() * (b - a);
// Arm the recorder.
const expr = fs.readFileSync(new URL('./capture_trace_expression.js', import.meta.url), 'utf8');
await cdp('Runtime.evaluate', { expression: expr, returnByValue: true });
const vw = tab.width || 1440, vh = (tab.height || 1000) - 100;
let x = vw * 0.3, y = vh * 0.4;

// Move along a smooth eased path to a target, with jitter and human pacing.
async function glide(tx, ty) {
  const steps = Math.floor(rand(16, 30));
  for (let s = 1; s <= steps; s++) {
    const t = s / steps;
    const e = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
    const cx = x + (tx - x) * e + rand(-2, 2);
    const cy = y + (ty - y) * e + rand(-2, 2);
    await cdp('Input.dispatchMouseEvent', { type: 'mouseMoved', x: cx, y: cy });
    await sleep(rand(7, 20));
  }
  x = tx; y = ty;
}
async function click() {
  const cx = x + rand(-3, 3), cy = y + rand(-3, 3);
  await cdp('Input.dispatchMouseEvent', { type: 'mousePressed', x: cx, y: cy, button: 'left', clickCount: 1 });
  await sleep(rand(60, 130));
  await cdp('Input.dispatchMouseEvent', { type: 'mouseReleased', x: cx, y: cy, button: 'left', clickCount: 1 });
}
// ~40s of varied activity: glide between waypoints, occasional clicks and
// scrolls, with rests like a person reading.
const phases = 8;
for (let p = 0; p < phases; p++) {
  const hops = Math.floor(rand(3, 6));
  for (let h = 0; h < hops; h++) {
    await glide(rand(vw * 0.15, vw * 0.85), rand(vh * 0.2, vh * 0.8));
    await sleep(rand(60, 180));
  }
  if (p % 3 === 1) { await click(); await sleep(rand(200, 500)); }
  if (p % 2 === 0) {
    await cdp('Input.dispatchMouseEvent', { type: 'mouseWheel', x, y, deltaX: 0, deltaY: (Math.random() < 0.5 ? -1 : 1) * Math.floor(rand(120, 400)) });
    await sleep(rand(300, 700));
  }
  await sleep(rand(700, 1800)); // rest, like reading
}
const read = await cdp('Runtime.evaluate', { expression: 'window.__traceStop ? window.__traceStop() : []', returnByValue: true });
const rec = read.result && read.result.result && read.result.result.value;
const out = new URL('./interaction-trace-long.json', import.meta.url).pathname;
fs.writeFileSync(out, JSON.stringify({ capturedAt: new Date().toISOString(), source: 'devtools-input-domain-long', events: rec || [] }));
const byType = {};
(rec || []).forEach(e => byType[e.type] = (byType[e.type] || 0) + 1);
const span = rec && rec.length ? rec[rec.length - 1].dt : 0;
console.log(JSON.stringify({ saved: out, events: (rec || []).length, spanMs: Math.round(span), byType }));
ws.close(); process.exit(0);
