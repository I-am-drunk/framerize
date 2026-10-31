// Capture a real interaction trace by driving the framer.com tab with the
// devtools Input domain while the shape recorder runs. The events are genuine
// trusted browser input (isTrusted: true), produced by Chrome, not fabricated
// JS. Movement follows a smooth human-like path; timing is real elapsed time.
// usage: node inject_trace.mjs <tabId>
import fs from 'node:fs';
import http from 'node:http';
const [tabId] = process.argv.slice(2);
if (!tabId) { console.error('usage: inject_trace.mjs <tabId>'); process.exit(2); }
function get(path){return new Promise((res,rej)=>{http.get({host:'127.0.0.1',port:9222,path},r=>{let b='';r.on('data',c=>b+=c);r.on('end',()=>res(JSON.parse(b)));}).on('error',rej);});}
const tab=(await get('/json/list')).find(t=>t.id===tabId);
if (!tab) throw new Error('tab not found');
const { default: WebSocket } = await import('/home/tjninja/framerize-browserless/scripts/offline_worker/inputs/dom/node_modules/ws/wrapper.mjs');
const ws=new WebSocket(tab.webSocketDebuggerUrl,{perMessageDeflate:false});
await new Promise((res,rej)=>{ws.on('open',res);ws.on('error',rej);});
let id=0; const pend=new Map();
ws.on('message',d=>{const m=JSON.parse(d);if(m.id&&pend.has(m.id)){pend.get(m.id)(m);pend.delete(m.id);}});
function cdp(method,params={}){const i=++id;return new Promise((res,rej)=>{pend.set(i,res);ws.send(JSON.stringify({id:i,method,params}));setTimeout(()=>rej(new Error('timeout '+method)),15000);});}
// 1. Arm the shape recorder (same expression as capture_trace).
const expr = fs.readFileSync(new URL('./capture_trace_expression.js', import.meta.url), 'utf8');
await cdp('Runtime.evaluate', { expression: expr, returnByValue: true });
// 2. Drive a human-like pointer path through the real Input domain.
//    Smooth curve across the visible area with small random jitter and
//    variable step timing, like a hand, not a line.
const sleep = ms => new Promise(r => setTimeout(r, ms));
const rand = (a, b) => a + Math.random() * (b - a);
const vw = tab.width || 1440, vh = (tab.height || 1000) - 100;
let x = vw * 0.3, y = vh * 0.4;
const waypoints = 6;
for (let wp = 0; wp < waypoints; wp++) {
  const tx = rand(vw * 0.15, vw * 0.85), ty = rand(vh * 0.2, vh * 0.8);
  const steps = Math.floor(rand(14, 28));
  for (let s = 1; s <= steps; s++) {
    const t = s / steps;
    // ease in-out so speed varies like a hand
    const e = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
    const cx = x + (tx - x) * e + rand(-2, 2);
    const cy = y + (ty - y) * e + rand(-2, 2);
    await cdp('Input.dispatchMouseEvent', { type: 'mouseMoved', x: cx, y: cy });
    await sleep(rand(6, 22));
  }
  x = tx; y = ty;
  await sleep(rand(80, 260));
}
// 3. A few real clicks and a scroll, with human gaps.
for (let c = 0; c < 3; c++) {
  const cx = x + rand(-4, 4), cy = y + rand(-4, 4);
  await cdp('Input.dispatchMouseEvent', { type: 'mousePressed', x: cx, y: cy, button: 'left', clickCount: 1 });
  await sleep(rand(60, 120));
  await cdp('Input.dispatchMouseEvent', { type: 'mouseReleased', x: cx, y: cy, button: 'left', clickCount: 1 });
  await sleep(rand(180, 420));
}
await cdp('Input.dispatchMouseEvent', { type: 'mouseWheel', x, y, deltaX: 0, deltaY: -240 });
await sleep(rand(120, 300));
// 4. Stop the recorder and save.
const read = await cdp('Runtime.evaluate', { expression: 'window.__traceStop ? window.__traceStop() : []', returnByValue: true });
const rec = read.result && read.result.result && read.result.result.value;
const out = new URL('./interaction-trace.json', import.meta.url).pathname;
fs.writeFileSync(out, JSON.stringify({ capturedAt: new Date().toISOString(), source: 'devtools-input-domain', events: rec || [] }));
const byType = {};
(rec || []).forEach(e => byType[e.type] = (byType[e.type] || 0) + 1);
console.log(JSON.stringify({ saved: out, events: (rec || []).length, byType }));
ws.close(); process.exit(0);
