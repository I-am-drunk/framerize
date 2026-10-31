// One-time capture: run the pinned Castle SDK graph in the user's real
// Chrome from a loopback origin, instrument 2D-context reads, and mint one
// genuine token. Assets are local pinned bytes with CDN URLs rewritten to
// the loopback server. No external network is contacted.
import fs from 'node:fs';
import http from 'node:http';

const tabId = process.argv[2];
if (!tabId) { console.error('usage: capture_probes.mjs <tabId>'); process.exit(2); }
const OUT = new URL('./probe-capture.json', import.meta.url).pathname;
const ASSETS = new URL('./local_assets/', import.meta.url).pathname;
const PORT = 18711;

function httpGet(path) {
  return new Promise((resolve, reject) => {
    http.get({ host: '127.0.0.1', port: 9222, path }, res => {
      let b = ''; res.on('data', c => b += c); res.on('end', () => resolve(JSON.parse(b)));
    }).on('error', reject);
  });
}
const targets = await httpGet('/json/list');
const tab = targets.find(t => t.id === tabId);
if (!tab) throw new Error('tab not found');

const { default: WebSocket } = await import(new URL('../inputs/dom/node_modules/ws/wrapper.mjs', import.meta.url).href);
const ws = new WebSocket(tab.webSocketDebuggerUrl, { perMessageDeflate: false });
await new Promise((res, rej) => { ws.on('open', res); ws.on('error', rej); });
let msgId = 0;
const pending = new Map();
ws.on('message', data => {
  const msg = JSON.parse(data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
});
function cdp(method, params = {}) {
  const id = ++msgId;
  return new Promise((resolve, reject) => {
    pending.set(id, resolve);
    ws.send(JSON.stringify({ id, method, params }));
    setTimeout(() => reject(new Error('cdp timeout ' + method)), 30000);
  });
}

const server = http.createServer((req, res) => {
  const name = req.url.replace(/^\//, '');
  if (name === '' || name === 'index.html') {
    res.writeHead(200, { 'content-type': 'text/html' });
    res.end('<!doctype html><html><head><meta charset=utf-8></head><body></body></html>');
    return;
  }
  const file = ASSETS + name;
  if (!fs.existsSync(file)) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { 'content-type': 'text/javascript' });
  fs.createReadStream(file).pipe(res);
});
await new Promise(r => server.listen(PORT, '127.0.0.1', r));

await cdp('Page.enable');
await cdp('Page.navigate', { url: `http://127.0.0.1:${PORT}/index.html` });
await new Promise(r => setTimeout(r, 1500));
const instrument = fs.readFileSync(new URL('./probe_instrument.js', import.meta.url), 'utf8');
const evalResult = await cdp('Runtime.evaluate', {
  expression: instrument, returnByValue: true, awaitPromise: true,
});
server.close();
ws.close();
const v = evalResult.result && evalResult.result.result && evalResult.result.result.value;
if (!v) {
  console.log('DEBUG ' + JSON.stringify(evalResult.result && evalResult.result.exceptionDetails).slice(0, 900));
  throw new Error('no capture');
}
fs.writeFileSync(OUT, JSON.stringify(v, null, 1));
console.log('reads=' + (v.reads ? v.reads.length : 0),
  'tokenLen=' + (v.tokenLength || 0), 'mintError=' + v.mintError,
  'errors=' + (v.errors ? v.errors.length : 0));
process.exit(0);
