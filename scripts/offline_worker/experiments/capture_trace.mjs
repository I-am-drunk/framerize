// Drive the trace capture in the user's framer.com tab over CDP.
// Arm the recorder, wait a bounded window for real interaction, then read it.
// Read-only; no navigation, no writes, tab untouched after.
// usage: node capture_trace.mjs <tabId> <seconds>
import fs from 'node:fs';
import http from 'node:http';
const [tabId, secs] = process.argv.slice(2);
if (!tabId || !secs) { console.error('usage: capture_trace.mjs <tabId> <seconds>'); process.exit(2); }
function get(path){return new Promise((res,rej)=>{http.get({host:'127.0.0.1',port:9222,path},r=>{let b='';r.on('data',c=>b+=c);r.on('end',()=>res(JSON.parse(b)));}).on('error',rej);});}
const tab=(await get('/json/list')).find(t=>t.id===tabId);
if (!tab) throw new Error('tab not found');
const { default: WebSocket } = await import('/home/tjninja/framerize-browserless/scripts/offline_worker/inputs/dom/node_modules/ws/wrapper.mjs');
const ws=new WebSocket(tab.webSocketDebuggerUrl,{perMessageDeflate:false});
await new Promise((res,rej)=>{ws.on('open',res);ws.on('error',rej);});
let id=0; const pend=new Map();
ws.on('message',d=>{const m=JSON.parse(d);if(m.id&&pend.has(m.id)){pend.get(m.id)(m);pend.delete(m.id);}});
function cdp(method,params={}){const i=++id;return new Promise((res,rej)=>{pend.set(i,res);ws.send(JSON.stringify({id:i,method,params}));setTimeout(()=>rej(new Error('timeout '+method)),15000);});}
const expr = fs.readFileSync(new URL('./capture_trace_expression.js', import.meta.url), 'utf8');
const arm = await cdp('Runtime.evaluate', { expression: expr, returnByValue: true });
console.error('armed:', JSON.stringify(arm.result && arm.result.result && arm.result.result.value));
const waitMs = Math.min(parseInt(secs, 10) || 20, 120) * 1000;
await new Promise(r => setTimeout(r, waitMs));
const read = await cdp('Runtime.evaluate', { expression: 'window.__traceStop ? window.__traceStop() : []', returnByValue: true, awaitPromise: true });
const rec = read.result && read.result.result && read.result.result.value;
const out = new URL('./interaction-trace.json', import.meta.url).pathname;
fs.writeFileSync(out, JSON.stringify({ capturedAt: new Date().toISOString(), events: rec || [] }));
const byType = {};
(rec || []).forEach(e => byType[e.type] = (byType[e.type] || 0) + 1);
console.log(JSON.stringify({ saved: out, events: (rec || []).length, byType }));
ws.close(); process.exit(0);
