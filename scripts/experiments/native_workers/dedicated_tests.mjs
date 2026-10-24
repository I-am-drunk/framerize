import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture, message, event, script } from './fixture.mjs';

test('dedicated workers keep independent state and native identity', {timeout:5000}, async t => {
  const {window, bridge, url} = fixture(t);
  const first = new window.Worker(url), second = new window.Worker(url);
  for (const worker of [first, second])
    assert.deepEqual(await message(worker), [1, 'Framerize/0.1', 'undefined']);
  assert.throws(() => new window.Worker(url));
  await bridge.close();
  assert.deepEqual(bridge.stats(), {started:2, joined:2, errors:0, messages:2});
});
test('different source refuses before starting a thread', {timeout:5000}, async t => {
  const {window, bridge, url} = fixture(t, script+';', script);
  const failed = await event(new window.Worker(url), 'error');
  assert.equal(failed.message, 'worker failed');
  await bridge.close();
  assert.deepEqual(bridge.stats(), {started:0, joined:0, errors:1, messages:0});
});
test('revoked, external and oversized worker sources refuse', async t => {
  const {window, bridge, url} = fixture(t);
  for (const path of ['https://framer.com/worker.js', 'file:///tmp/worker.js'])
    assert.throws(() => new window.Worker(path));
  assert.throws(() => window.URL.createObjectURL(new window.Blob(['x'.repeat(4097)], {type:'application/javascript'})));
  window.URL.revokeObjectURL(url);
  assert.throws(() => new window.Worker(url));
  await bridge.close();
  assert.equal(bridge.stats().started, 0);
});
test('close during preparation prevents launch', async t => {
  const {window, bridge, url} = fixture(t);
  const worker = new window.Worker(url);
  worker.terminate(); worker.terminate();
  const closing = bridge.close();
  assert.equal(closing, bridge.close());
  await closing;
  assert.deepEqual(bridge.stats(), {started:0, joined:0, errors:0, messages:0});
});
test('runtime errors omit private text and are joined', {timeout:5000}, async t => {
  const {window, bridge, url} = fixture(t, "throw Error('fixture-private-marker')");
  assert.equal((await event(new window.Worker(url), 'error')).message, 'worker failed');
  await bridge.close();
  assert.deepEqual(bridge.stats(), {started:1, joined:1, errors:1, messages:0});
});
test('termination joins a worker executing JavaScript', {timeout:5000}, async t => {
  const {window, bridge, url} = fixture(t, "self.onmessage=()=>{postMessage('started');for(;;){}}");
  const worker = new window.Worker(url);
  assert.equal(await message(worker), 'started');
  worker.terminate();
  await bridge.close();
  assert.deepEqual(bridge.stats(), {started:1, joined:1, errors:0, messages:1});
});
test('only one empty input message is accepted', {timeout:5000}, async t => {
  const {window, url} = fixture(t);
  const worker = new window.Worker(url);
  assert.throws(() => worker.postMessage('private'));
  assert.deepEqual(await message(worker), [1, 'Framerize/0.1', 'undefined']);
  assert.throws(() => worker.postMessage(''));
});
test('SDK reply classes are counted without retaining device fields', {timeout:5000}, async t => {
  const source = `self.onmessage=()=>{const a=new Array(10);
    a[0]=1;a[1]=true;a[2]=navigator.userAgent;a[7]=[];
    postMessage(a);postMessage([3,false,null]);postMessage([4,false,1]);}`;
  const {window, bridge, url} = fixture(t, source);
  const worker = new window.Worker(url);
  const finished = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('reply timeout')), 2500);
    worker.onmessage = () => {
      if (bridge.stats().messages === 3) { clearTimeout(timer); resolve(); }
    };
  });
  worker.postMessage(''); await finished; await bridge.close();
  assert.deepEqual(bridge.replyStats(), {feature:1, entropy:1, timing:1, error:0, other:0});
});
