'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { Script, createContext } = require('node:vm');
const { TextDecoder } = require('node:util');

function script(name) {
  const path = join(__dirname, name);
  return new Script(`(${readFileSync(path, 'utf8')})`, { filename: path });
}
const readerProgram = script('queue_reader.js'), pollProgram = script('queue_poll.js');
const capacity = 131072;

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

async function until(predicate, label) {
  for (let turn = 0; turn < 256; turn++) {
    if (predicate()) return;
    await Promise.resolve();
  }
  assert.fail(`no bounded progress: ${label}`);
}

function controlledReader() {
  const pending = [], cancelled = deferred();
  const calls = { reads: 0, cancels: 0, releases: 0, locks: 0 };
  const reader = {
    read() { calls.reads++; const next = deferred(); pending.push(next); return next.promise; },
    cancel() { calls.cancels++; return cancelled.promise; },
    releaseLock() { calls.releases++; },
  };
  return { reader, pending, cancelled, calls };
}

function start({ status = 200, usage = 'fixture-usage', byteLimit = capacity * 4,
                 body = true, alreadyAborted = false } = {}) {
  const flow = controlledReader(), controller = new AbortController();
  const job = { done: false, timer: { label: 'target timer' } };
  const window = { __aiJobs: { target: job } }, cleared = [];
  const context = createContext({ window, TextDecoder,
    clearTimeout: timer => cleared.push(timer) });
  const response = { status, headers: { get: name => {
    assert.equal(name, 'x-framer-ai-usage'); return usage;
  } }, body: body ? { getReader() { flow.calls.locks++; return flow.reader; } } : null };
  const f = { flow, controller, job, window, cleared, status, usage,
    settled: false, outcome: null, poll: pollProgram.runInContext(context) };
  const run = readerProgram.runInContext(context);
  if (alreadyAborted) controller.abort();
  f.joined = run(response, controller, job, byteLimit).then(
    value => { f.settled = true; return f.outcome = { ok: true, value }; },
    error => { f.settled = true; return f.outcome = { ok: false, error }; },
  );
  return f;
}

async function feed(f, value, done = false) {
  await until(() => f.flow.pending.length > 0, 'pending reader.read');
  f.flow.pending.shift().resolve({ value, done });
}

async function joinProducer(f) {
  await until(() => f.settled, 'joined producer settlement');
  return f.joined;
}

function publish(f) {
  assert.equal(f.settled, true, 'the fixture publishes only a joined outcome');
  f.job.value = f.outcome.ok
    ? { status: f.status, usage: f.usage, body: f.outcome.value }
    : { status: -1, body: 'fixture refusal' };
  f.job.done = true;
}

function finalPoll(f) {
  publish(f);
  const result = f.poll('target');
  assert.equal(result.done, true);
  assert.equal(Object.hasOwn(f.window.__aiJobs, 'target'), false);
  assert.deepEqual(f.cleared, [f.job.timer]);
  assert.equal(f.poll('target'), null);
  assert.deepEqual(f.cleared, [f.job.timer]);
  return result;
}

test('capacity backpressure resumes only after poll frees queue room', async () => {
  const f = start(), bytes = Buffer.alloc(capacity + 32768, 97);
  await feed(f, bytes);
  await until(() => typeof f.job.queue.resume === 'function', 'queue backpressure');
  assert.equal(f.job.queue.capacity, capacity);
  assert.equal(f.job.queue.units, capacity);
  assert.equal(f.job.queue.peakUnits, capacity);
  assert.equal(f.flow.calls.reads, 1);
  assert.equal(f.settled, false);
  assert.equal(f.flow.calls.releases, 0);
  const first = f.poll('target');
  assert.equal(first.body, 'a'.repeat(capacity));
  assert.equal(first.offset, 0);
  assert.equal(first.next, capacity);
  assert.equal(first.done, false);
  assert.equal(f.window.__aiJobs.target, f.job);
  assert.deepEqual(f.cleared, []);
  await until(() => f.flow.pending.length === 1, 'producer resumes its reads');
  assert.equal(f.job.queue.units, 32768);
  await feed(f, undefined, true);
  assert.deepEqual(await joinProducer(f), { ok: true, value: '' });
  assert.equal(f.flow.calls.releases, 1);
  assert.equal(f.flow.calls.cancels, 0);
  const last = finalPoll(f);
  assert.equal(last.body, 'a'.repeat(32768));
  assert.equal(last.offset, capacity);
  assert.equal(last.next, bytes.length);
  assert.equal(last.received_bytes, bytes.length);
  assert.equal(last.peak_units, capacity);
  assert.equal(first.body + last.body, bytes.toString('utf8'));
});

for (const splitReads of [false, true]) {
  test(`UTF-8 survives ${splitReads ? 'split reads' : 'internal slices'} and poll`, async () => {
    const expected = 'A'.repeat(32767) + '\u{1F680}' + 'Z';
    const bytes = Buffer.from(expected, 'utf8'), f = start(), received = [];
    const inputs = splitReads
      ? [bytes.subarray(0, 32768), bytes.subarray(32768, 32769), bytes.subarray(32769)]
      : [bytes];
    for (const input of inputs) {
      await feed(f, input);
      await until(() => f.flow.pending.length === 1, 'next UTF-8 input read');
      const part = f.poll('target');
      if (part) {
        const decoded = Buffer.from(part.body, 'utf8').toString('utf8');
        assert.equal(decoded, part.body);
        assert.equal(part.done, false);
        received.push(decoded);
      }
    }
    await feed(f, undefined, true);
    assert.deepEqual(await joinProducer(f), { ok: true, value: '' });
    const last = finalPoll(f);
    received.push(Buffer.from(last.body, 'utf8').toString('utf8'));
    assert.equal(received.join(''), expected);
    assert.equal(last.next, expected.length);
    assert.equal(last.received_bytes, bytes.length);
    assert.equal(f.flow.calls.releases, 1);
    assert.equal(f.flow.calls.cancels, 0);
  });
}

test('the byte limit includes a body exactly at its UTF-8 byte count', async () => {
  const expected = '\u00e9\u{1F680}Z', bytes = Buffer.from(expected, 'utf8');
  const f = start({ byteLimit: bytes.length });
  await feed(f, bytes);
  await feed(f, undefined, true);
  assert.deepEqual(await joinProducer(f), { ok: true, value: '' });
  const last = finalPoll(f);
  assert.equal(last.body, expected);
  assert.equal(last.received_bytes, bytes.length);
  assert.equal(f.controller.signal.aborted, false);
  assert.equal(f.flow.calls.cancels, 0);
  assert.equal(f.flow.calls.releases, 1);
});

test('oversize clears queued text and joins cancellation even when cancel rejects', async () => {
  const f = start({ byteLimit: 5 });
  await feed(f, Buffer.from('ab'));
  await until(() => f.flow.pending.length === 1, 'queued prefix');
  assert.equal(f.job.queue.parts.join(''), 'ab');
  await feed(f, Buffer.from('\u{1F680}', 'utf8'));
  await until(() => f.flow.calls.cancels === 1, 'oversize cancellation');
  assert.equal(f.job.queue.received, 6, 'the limit counts bytes, not UTF-16 units');
  assert.equal(f.controller.signal.aborted, true);
  assert.equal(f.job.queue.parts.length, 0);
  assert.equal(f.job.queue.units, 0);
  assert.equal(f.poll('target'), null);
  assert.equal(f.settled, false);
  assert.equal(f.flow.calls.releases, 0);
  f.flow.cancelled.reject(new Error('fixture cancel refusal'));
  const outcome = await joinProducer(f);
  assert.equal(outcome.ok, false);
  assert.equal(outcome.error.message, 'response body limit exceeded');
  assert.equal(f.flow.calls.releases, 1);
  assert.equal(finalPoll(f).status, -1);
});

test('abort wakes a producer waiting for queue room and joins its reader', async () => {
  const f = start();
  await feed(f, Buffer.alloc(capacity + 32768, 97));
  await until(() => typeof f.job.queue.resume === 'function', 'full queue');
  f.controller.abort();
  await until(() => f.flow.calls.cancels === 1, 'abort cancellation');
  assert.equal(f.job.queue.resume, null);
  assert.equal(f.job.queue.parts.length, 0);
  assert.equal(f.job.queue.units, 0);
  assert.equal(f.flow.calls.reads, 1);
  assert.equal(f.flow.calls.releases, 0);
  assert.equal(f.settled, false);
  assert.equal(f.poll('target'), null);
  f.flow.cancelled.resolve();
  const outcome = await joinProducer(f);
  assert.equal(outcome.ok, false);
  assert.equal(outcome.error.message, 'response cancelled');
  assert.equal(f.flow.calls.releases, 1);
  assert.equal(finalPoll(f).status, -1);
});

test('EOF observed after abort cannot publish a successful result', async () => {
  const f = start();
  await until(() => f.flow.pending.length === 1, 'EOF read');
  f.flow.pending.shift().resolve({ done: true });
  f.controller.abort();
  await until(() => f.flow.calls.cancels === 1, 'aborted EOF cancellation');
  assert.equal(f.settled, false);
  assert.equal(f.flow.calls.releases, 0);
  f.flow.cancelled.resolve();
  const outcome = await joinProducer(f);
  assert.equal(outcome.ok, false);
  assert.equal(outcome.error.message, 'response cancelled');
  assert.equal(f.flow.calls.releases, 1);
  assert.equal(finalPoll(f).status, -1);
});

for (const item of [
  { name: 'non-200 body', status: 503, usage: 'fixture-limit', text: 'unavailable', body: true },
  { name: 'empty stream', status: 204, usage: null, text: '', body: true },
  { name: 'absent body', status: 205, usage: null, text: '', body: false },
]) {
  test(`status and usage survive ${item.name}`, async () => {
    const f = start(item);
    if (item.body) {
      if (item.text) await feed(f, Buffer.from(item.text));
      await feed(f, undefined, true);
    }
    assert.deepEqual(await joinProducer(f), { ok: true, value: '' });
    const last = finalPoll(f);
    assert.equal(last.status, item.status);
    assert.equal(last.usage, item.usage);
    assert.equal(last.body, item.text);
    assert.equal(last.received_bytes, Buffer.byteLength(item.text));
    assert.equal(f.flow.calls.locks, Number(item.body));
    assert.equal(f.flow.calls.releases, Number(item.body));
    assert.equal(f.flow.calls.cancels, 0);
  });
}

test('missing polls and target completion preserve an unrelated job and timer', async () => {
  const f = start(), otherTimer = { label: 'unrelated timer' };
  const other = { done: false, timer: otherTimer, value: { body: 'untouched' } };
  const before = { ...other, value: { ...other.value } };
  f.window.__aiJobs.other = other;
  const retained = () => {
    assert.equal(f.window.__aiJobs.other, other);
    assert.deepEqual(other, before);
    assert.ok(!f.cleared.includes(otherTimer));
  };
  assert.equal(f.poll('missing'), null);
  assert.equal(f.poll('target'), null);
  retained();
  await feed(f, Buffer.from('target content'));
  await until(() => f.flow.pending.length === 1, 'pending target EOF');
  assert.equal(f.poll('target').body, 'target content');
  retained();
  await feed(f, undefined, true);
  assert.deepEqual(await joinProducer(f), { ok: true, value: '' });
  assert.equal(f.flow.calls.releases, 1);
  assert.equal(finalPoll(f).body, '');
  retained();
  assert.deepEqual(Object.keys(f.window.__aiJobs), ['other']);
});

test('an already-aborted absent body cannot publish success', async () => {
  const f = start({ body: false, alreadyAborted: true });
  const outcome = await joinProducer(f);
  assert.equal(outcome.ok, false);
  assert.equal(outcome.error.message, 'response cancelled');
  assert.equal(f.flow.calls.locks, 0);
  assert.equal(f.flow.calls.releases, 0);
  assert.equal(finalPoll(f).status, -1);
});
