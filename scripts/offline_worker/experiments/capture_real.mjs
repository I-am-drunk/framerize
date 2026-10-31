// Drive the capture expression in the user's real Chrome tab over CDP.
// Read-only evaluation; no navigation, no storage writes, no tokens.
import fs from 'node:fs';
import http from 'node:http';

const tabId = process.argv[2];
if (!tabId) { console.error('usage: capture_real.mjs <tabId>'); process.exit(2); }
const OUT = new URL('./real-browser-values.json', import.meta.url).pathname;

function get(path) {
  return new Promise((resolve, reject) => {
    http.get({ host: '127.0.0.1', port: 9222, path }, res => {
      let b = '';
      res.on('data', c => b += c);
      res.on('end', () => resolve(JSON.parse(b)));
    }).on('error', reject);
  });
}
const targets = await get('/json/list');
const tab = targets.find(t => t.id === tabId);
if (!tab) throw new Error('tab not found');

const { default: WebSocket } = await import(new URL('../inputs/dom/node_modules/ws/wrapper.mjs', import.meta.url).href);
if (!WebSocket) throw new Error('ws module unavailable');
const ws = new WebSocket(tab.webSocketDebuggerUrl, { perMessageDeflate: false });
await new Promise((res, rej) => { ws.on('open', res); ws.on('error', rej); });

const expression = fs.readFileSync(new URL('./capture_expression.js', import.meta.url), 'utf8');
const reply = await new Promise((resolve, reject) => {
  const id = 1;
  ws.on('message', data => {
    const msg = JSON.parse(data);
    if (msg.id === id) resolve(msg);
  });
  ws.on('error', reject);
  ws.send(JSON.stringify({
    id, method: 'Runtime.evaluate',
    params: { expression, returnByValue: true, awaitPromise: true },
  }));
});
ws.close();
const value = reply.result && reply.result.result && reply.result.result.value;
if (!value) throw new Error('no capture value: ' + JSON.stringify(reply).slice(0, 400));
fs.writeFileSync(OUT, JSON.stringify(value, null, 1));
console.log('captured', Object.keys(value).join(','));
