import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';
import { DOM } from './pins.mjs';
import { installSharedWorkers } from './shared_workers.mjs';
const { JSDOM, ResourceLoader } = createRequire(import.meta.url)(DOM + '/node_modules/jsdom/lib/api.js');
const source = `let count=0; self.onconnect=e=>{const p=e.ports[0];
p.onmessage=e=>p.postMessage([++count,e.data,navigator.userAgent,typeof navigator.userAgentData]);p.start()}`;
function fixture(t, script = source, expected = script) {
  const dom = new JSDOM('', { url:'https://framer.com/projects/test', runScripts:'outside-only',
    resources:new ResourceLoader({userAgent:'Framerize/0.1'}) });
  const bridge = installSharedWorkers(dom.window, createHash('sha256').update(expected).digest('hex'));
  const url = dom.window.URL.createObjectURL(new dom.window.Blob([script], {type:'application/javascript'}));
  t.after(async () => { await bridge.close(); dom.window.close(); });
  return { window:dom.window, bridge, url };
}
function message(port, value) {
  return new Promise((resolve, reject) => {
    const timer=setTimeout(()=>reject(new Error('message timeout')),2500);
    port.onmessage=e=>{clearTimeout(timer);resolve(e.data)}; port.start(); port.postMessage(value);
  });
}
function failure(worker) {
  return new Promise((resolve, reject) => {
    const timer=setTimeout(()=>reject(new Error('error timeout')),2500);
    worker.onerror=e=>{clearTimeout(timer);resolve(e.message)};
  });
}
test('two ports share real worker state and the truthful native UA', {timeout:5000}, async t=>{
  const {window,bridge,url}=fixture(t);
  const a=new window.SharedWorker(url), b=new window.SharedWorker(url);
  assert.deepEqual(await message(a.port,'first'),[1,'first','Framerize/0.1','undefined']);
  assert.deepEqual(await message(b.port,'second'),[2,'second','Framerize/0.1','undefined']);
  assert.equal(bridge.stats().started,1); await bridge.close();
  assert.deepEqual(bridge.stats(),{started:1,joined:1,errors:0,connections:2,messages:2,sdk_replies:0});
});
test('different windows keep separate worker state', {timeout:5000}, async t=>{
  const a=fixture(t),b=fixture(t);
  const first=new a.window.SharedWorker(a.url),second=new b.window.SharedWorker(b.url);
  assert.equal((await message(first.port,'a'))[0],1);
  assert.equal((await message(second.port,'b'))[0],1);
  await Promise.all([a.bridge.close(),b.bridge.close()]);
  assert.equal(a.bridge.stats().joined,1);assert.equal(b.bridge.stats().joined,1);
});
test('changed source refuses before worker launch', {timeout:5000}, async t=>{
  const {window,bridge,url}=fixture(t,source+';',source);
  assert.equal(await failure(new window.SharedWorker(url)),'shared worker failed');
  await bridge.close();
  assert.deepEqual(bridge.stats(),{started:0,joined:0,errors:1,connections:1,messages:0,sdk_replies:0});
});
test('external, revoked, oversized and unsupported resources refuse', t=>{
  const {window,bridge,url}=fixture(t);
  for(const path of ['https://framer.com/worker.js','file:///tmp/worker.js','blob:https://framer.com/unknown'])
    assert.throws(()=>new window.SharedWorker(path));
  assert.throws(()=>window.URL.createObjectURL(new window.Blob(['x'],{type:'text/plain'})));
  assert.throws(()=>window.URL.createObjectURL(new window.Blob(['x'.repeat(4097)],{type:'application/javascript'})));
  window.URL.revokeObjectURL(url);assert.throws(()=>new window.SharedWorker(url));
  assert.equal(bridge.stats().started,0);
});
test('close during preparation prevents launch and is idempotent', async t=>{
  const {window,bridge,url}=fixture(t);new window.SharedWorker(url);
  const first=bridge.close();assert.equal(first,bridge.close());await first;
  assert.equal(bridge.stats().started,0);assert.equal(bridge.stats().joined,0);
  assert.throws(()=>new window.SharedWorker(url));
});
test('worker failure reports no source text and joins', {timeout:5000}, async t=>{
  const {window,bridge,url}=fixture(t,"throw Error('fixture-private-marker')");
  assert.equal(await failure(new window.SharedWorker(url)),'shared worker failed');
  await bridge.close();
  assert.deepEqual(bridge.stats(),{started:1,joined:1,errors:1,connections:1,messages:0,sdk_replies:0});
});
test('per-window worker and connection limits cannot be raced', async t=>{
  const {window,bridge,url}=fixture(t);
  const second=window.URL.createObjectURL(new window.Blob([source],{type:'application/javascript'}));
  new window.SharedWorker(url);
  assert.throws(()=>new window.SharedWorker(second));
  for(let n=0;n<3;n++)new window.SharedWorker(url);
  assert.throws(()=>new window.SharedWorker(url));
  await bridge.close();assert.equal(bridge.stats().connections,4);
});
test('close joins a worker actively executing JavaScript', {timeout:5000}, async t=>{
  const script="self.onconnect=e=>{const p=e.ports[0];p.onmessage=()=>{p.postMessage('started');for(;;){}};p.start()}";
  const {window,bridge,url}=fixture(t,script);
  const worker=new window.SharedWorker(url);
  assert.equal(await message(worker.port,'run'),'started');
  await bridge.close();
  assert.deepEqual(bridge.stats(),{started:1,joined:1,errors:0,connections:1,messages:1,sdk_replies:0});
});
