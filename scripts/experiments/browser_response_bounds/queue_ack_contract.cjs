'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { Script, createContext } = require('node:vm');
const { TextDecoder } = require('node:util');

function program(name) {
  const path = join(__dirname, name);
  return new Script(`(${readFileSync(path, 'utf8')})`, { filename: path });
}
const readerProgram = program('queue_reader.js');
const pollProgram = program('queue_ack_poll.js'), capacity = 131072;

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

function fixture() {
  const controller = new AbortController();
  const job = { done: false, controller, timer: { label: 'target' } };
  const other = { done: false, timer: { label: 'other' }, value: { body: 'untouched' } };
  const window = { __aiJobs: { target: job, other } }, cleared = [];
  const context = createContext({ window, TextDecoder,
    clearTimeout: timer => cleared.push(timer) });
  return { controller, job, other, window, cleared,
    otherBefore: JSON.stringify(other), read: readerProgram.runInContext(context),
    poll: pollProgram.runInContext(context) };
}

function start({ status = 200, usage = 'fixture-usage' } = {}) {
  const f = fixture(), pending = [], cancelled = deferred();
  const calls = { reads: 0, cancels: 0, releases: 0, locks: 0 };
  const reader = {
    read() { calls.reads++; const next = deferred(); pending.push(next); return next.promise; },
    cancel() { calls.cancels++; return cancelled.promise; },
    releaseLock() { calls.releases++; },
  };
  const response = { status, headers: { get: name => {
    assert.equal(name, 'x-framer-ai-usage'); return usage;
  } }, body: { getReader() { calls.locks++; return reader; } } };
  Object.assign(f, { pending, cancelled, calls, status, usage, settled: false });
  f.joined = f.read(response, f.controller, f.job, capacity * 4).then(
    value => { f.settled = true; return f.outcome = { ok: true, value }; },
    error => { f.settled = true; return f.outcome = { ok: false, error }; },
  );
  return f;
}

async function feed(f, value, done = false) {
  await until(() => f.pending.length === 1, 'pending reader.read');
  f.pending.shift().resolve({ value, done });
}

async function joinProducer(f) {
  await until(() => f.settled, 'joined producer');
  return f.joined;
}

async function finish(f) {
  await feed(f, undefined, true);
  assert.deepEqual(await joinProducer(f), { ok: true, value: '' });
  f.job.value = { status: f.status, usage: f.usage, body: f.outcome.value };
  f.job.done = true;
  assert.equal(f.calls.locks, 1);
  assert.equal(f.calls.releases, 1);
  assert.equal(f.calls.cancels, 0);
}

function frame(f, acknowledged = -1) {
  const result = f.poll(['target', acknowledged]);
  assert.equal(result.kind, 'chunk');
  assert.equal(Object.isFrozen(result), true);
  return result;
}

function pending(f, acknowledged = -1) {
  assert.deepEqual({ ...f.poll(['target', acknowledged]) }, { kind: 'pending' });
}

function otherIntact(f) {
  assert.equal(f.window.__aiJobs.other, f.other);
  assert.equal(JSON.stringify(f.other), f.otherBefore);
  assert.equal(f.cleared.includes(f.other.timer), false);
}

function close(f, last) {
  assert.equal(last.done, true);
  assert.equal(f.window.__aiJobs.target, f.job);
  assert.deepEqual(f.cleared, []);
  assert.deepEqual({ ...f.poll(['target', last.sequence]) },
    { kind: 'closed', sequence: last.sequence });
  assert.equal(Object.hasOwn(f.window.__aiJobs, 'target'), false);
  if (f.job.queue) assert.equal(f.job.queue.units, 0);
  for (const acknowledged of [last.sequence, -1]) {
    assert.deepEqual({ ...f.poll(['target', acknowledged]) }, { kind: 'missing' });
    assert.deepEqual(f.cleared, [f.job.timer]);
  }
  otherIntact(f);
}

function deliveryState(f) {
  const state = f.job.delivery, queue = f.job.queue;
  return { state, acknowledged: state?.acknowledged, offset: state?.offset,
    pending: state?.pending, frame: state?.pending?.frame,
    pendingUnits: state?.pending?.units, units: queue?.units,
    parts: queue ? Array.from(queue.parts) : null, resume: queue?.resume,
    peak: queue?.peakUnits, received: queue?.received,
    done: f.job.done, value: f.job.value, cleared: [...f.cleared] };
}

function refuses(f, acknowledged) {
  const before = deliveryState(f);
  assert.throws(() => f.poll(['target', acknowledged]), /invalid .*acknowledgement/);
  assert.equal(f.job.delivery, before.state);
  assert.deepEqual(deliveryState(f), before);
  otherIntact(f);
}

test('invalid initial acknowledgements leave delivery and ownership unchanged', async () => {
  const f = start();
  for (const value of [-2, 0.5, NaN, Infinity, -Infinity, Number.MAX_SAFE_INTEGER + 1,
                       '-1', null, undefined, true, false, 1n, 0, 1]) {
    refuses(f, value);
    assert.equal(Object.hasOwn(f.job, 'delivery'), false);
  }
  pending(f);
  await finish(f);
  close(f, frame(f));
});

test('pending and missing polls preserve an empty final frame until acknowledgement', async () => {
  const f = start({ status: 204, usage: null });
  assert.deepEqual({ ...f.poll(['absent', -1]) }, { kind: 'missing' });
  assert.equal(Object.hasOwn(f.job, 'delivery'), false);
  pending(f);
  pending(f);
  otherIntact(f);
  await finish(f);
  const last = frame(f);
  assert.deepEqual({ ...last }, { status: 204, usage: null, body: '',
    peak_units: 0, received_bytes: 0, kind: 'chunk', sequence: 0,
    done: true, offset: 0, next: 0 });
  assert.equal(frame(f), last);
  close(f, last);
});

test('a lost nonfinal reply replays without releasing its queue capacity', async () => {
  const f = start();
  await feed(f, Buffer.from('alpha'));
  await until(() => f.pending.length === 1, 'queued nonfinal data');
  const first = frame(f);
  assert.deepEqual({ ...first }, { status: 200, usage: 'fixture-usage', body: 'alpha',
    peak_units: 5, received_bytes: 5, kind: 'chunk', sequence: 0,
    done: false, offset: 0, next: 5 });
  for (let replay = 0; replay < 3; replay++) {
    assert.equal(frame(f), first);
    assert.equal(f.job.queue.units, 5);
    assert.equal(f.job.queue.parts.length, 0);
    assert.deepEqual(f.cleared, []);
  }
  pending(f, first.sequence);
  assert.equal(f.job.queue.units, 0);
  await finish(f);
  const last = frame(f, first.sequence);
  assert.equal(last.sequence, 1);
  assert.equal(last.offset, 5);
  assert.equal(last.next, 5);
  close(f, last);
});

test('a lost final reply retains status, usage, job and timer until acknowledgement', async () => {
  const f = start({ status: 503, usage: 'fixture-final-usage' });
  await feed(f, Buffer.from('final body'));
  await finish(f);
  const last = frame(f);
  assert.equal(last.done, true);
  assert.equal(last.status, 503);
  assert.equal(last.usage, 'fixture-final-usage');
  assert.equal(last.body, 'final body');
  assert.equal(last.offset, 0);
  assert.equal(last.next, 10);
  for (let replay = 0; replay < 3; replay++) {
    assert.equal(frame(f), last);
    assert.equal(f.job.queue.units, 10);
    assert.equal(f.window.__aiJobs.target, f.job);
    assert.deepEqual(f.cleared, []);
    otherIntact(f);
  }
  close(f, last);
});

test('acknowledgement plus lost next reply releases each frame only once', async () => {
  const f = start();
  await feed(f, Buffer.from('one'));
  await until(() => f.pending.length === 1, 'first queued frame');
  const first = frame(f);
  await feed(f, Buffer.from('twotwo'));
  await until(() => f.pending.length === 1, 'second queued frame');
  assert.equal(f.job.queue.units, 9);
  const second = frame(f, first.sequence);
  assert.equal(second.sequence, 1);
  assert.equal(second.offset, 3);
  assert.equal(second.next, 9);
  assert.equal(second.body, 'twotwo');
  for (let replay = 0; replay < 3; replay++) {
    assert.equal(frame(f, first.sequence), second);
    assert.equal(f.job.queue.units, 6);
  }
  await finish(f);
  assert.equal(frame(f, first.sequence), second);
  assert.equal(second.done, false);
  const last = frame(f, second.sequence);
  assert.equal(last.sequence, 2);
  assert.equal(last.offset, 9);
  assert.equal(last.next, 9);
  assert.equal(last.body, '');
  close(f, last);
});

test('unacknowledged data keeps the 131072-unit bound and producer backpressure', async () => {
  const f = start(), bytes = Buffer.alloc(capacity + 32768, 97);
  await feed(f, bytes);
  await until(() => typeof f.job.queue.resume === 'function', 'full queue');
  const resume = f.job.queue.resume, first = frame(f);
  assert.equal(first.body, 'a'.repeat(capacity));
  assert.equal(first.done, false);
  assert.equal(f.job.queue.parts.length, 0);
  assert.equal(f.job.queue.units, capacity);
  assert.equal(f.job.queue.peakUnits, capacity);
  assert.equal(frame(f), first);
  await Promise.resolve();
  assert.equal(f.job.queue.resume, resume);
  assert.equal(f.calls.reads, 1);
  assert.equal(f.calls.releases, 0);
  assert.equal(f.settled, false);
  pending(f, first.sequence);
  assert.equal(f.job.queue.units, 0);
  await until(() => f.pending.length === 1, 'producer released by acknowledgement');
  assert.equal(f.job.queue.units, 32768);
  assert.equal(f.job.queue.peakUnits, capacity);
  await finish(f);
  const last = frame(f, first.sequence);
  assert.equal(last.sequence, 1);
  assert.equal(last.offset, capacity);
  assert.equal(last.next, bytes.length);
  assert.equal(last.received_bytes, bytes.length);
  assert.equal(last.peak_units, capacity);
  assert.equal(first.body + last.body, bytes.toString('utf8'));
  close(f, last);
});

test('future and stale acknowledgements cannot mutate a pending or drained delivery', async () => {
  const f = start();
  await feed(f, Buffer.from('one'));
  await until(() => f.pending.length === 1, 'first refusal frame');
  const first = frame(f);
  refuses(f, 1);
  refuses(f, Number.MAX_SAFE_INTEGER);
  assert.equal(frame(f), first);
  await feed(f, Buffer.from('two'));
  await until(() => f.pending.length === 1, 'second refusal frame');
  const second = frame(f, first.sequence);
  for (const value of [-1, 2, Number.MAX_SAFE_INTEGER]) refuses(f, value);
  assert.equal(frame(f, first.sequence), second);
  pending(f, second.sequence);
  for (const value of [-1, 0, 2]) refuses(f, value);
  await finish(f);
  const last = frame(f, second.sequence);
  refuses(f, 0);
  refuses(f, 3);
  assert.equal(frame(f, second.sequence), last);
  close(f, last);
});

function assertFailureValue(actual, value) {
  // Forward the whole supplied failure record, including fields unknown to this poll.
  for (const [key, expected] of Object.entries(value)) assert.deepEqual(actual[key], expected);
  assert.deepEqual(Object.keys(actual).sort(),
    [...Object.keys(value), 'kind', 'sequence', 'done', 'offset', 'next'].sort());
  assert.equal(actual.status, -1);
  assert.equal(actual.done, true);
}

test('an early failure with no queue replays every supplied failure field', () => {
  const f = fixture();
  const value = { status: -1, body: 'fixture early failure', usage: 'fixture-usage',
    error: 'fixture error', fixture_detail: { joined: true, code: 'early' } };
  assert.equal(Object.hasOwn(f.job, 'delivery'), false);
  f.job.value = value;
  f.job.done = true;
  assert.equal(Object.hasOwn(f.job, 'queue'), false);
  const last = frame(f);
  assertFailureValue(last, value);
  assert.equal(last.sequence, 0);
  assert.equal(last.offset, 0);
  assert.equal(last.next, 0);
  assert.equal(frame(f), last);
  assert.equal(f.job.value, value);
  close(f, last);
});

for (const acknowledgeBeforeJoin of [true, false]) {
  test(`abort with a pending frame joins failure with acknowledgement ${
    acknowledgeBeforeJoin ? 'before' : 'after'} cancellation`, async () => {
    const f = start();
    await feed(f, Buffer.alloc(capacity + 32768, 97));
    await until(() => typeof f.job.queue.resume === 'function', 'pending abort frame');
    const first = frame(f);
    f.controller.abort();
    await until(() => f.calls.cancels === 1, 'reader cancellation');
    assert.equal(f.job.queue.units, 0);
    assert.equal(f.job.queue.parts.length, 0);
    assert.equal(f.job.queue.resume, null);
    assert.equal(f.calls.releases, 0);
    assert.equal(f.settled, false);
    assert.equal(f.job.done, false);
    assert.equal(frame(f), first);
    assert.equal(first.done, false);
    assert.deepEqual(f.cleared, []);
    if (acknowledgeBeforeJoin) pending(f, first.sequence);
    assert.equal(f.job.queue.units, 0);
    assert.equal(f.settled, false);
    f.cancelled.resolve();
    const outcome = await joinProducer(f);
    assert.equal(outcome.ok, false);
    assert.equal(outcome.error.message, 'response cancelled');
    assert.equal(f.calls.releases, 1);
    assert.equal(f.calls.cancels, 1);
    assert.equal(f.calls.locks, 1);
    const value = { status: -1, body: outcome.error.message, usage: null,
      error: 'fixture cancellation', fixture_detail: { cancel_joined: true, releases: 1 } };
    f.job.value = value;
    f.job.done = true;
    if (!acknowledgeBeforeJoin) assert.equal(frame(f), first);
    const last = frame(f, first.sequence);
    assertFailureValue(last, value);
    assert.equal(last.sequence, 1);
    assert.equal(last.offset, capacity);
    assert.equal(last.next, capacity);
    assert.equal(f.job.queue.units, 0);
    assert.equal(frame(f, first.sequence), last);
    close(f, last);
  });
}

for (const splitReads of [false, true]) {
  test(`UTF-8 survives acknowledged frame replay across ${
    splitReads ? 'split reads' : 'internal slices'}`, async () => {
    const expected = 'A'.repeat(32767) + '\u{1F680}' + 'Z';
    const bytes = Buffer.from(expected, 'utf8'), f = start(), received = [];
    const inputs = splitReads
      ? [bytes.subarray(0, 32768), bytes.subarray(32768, 32769), bytes.subarray(32769)]
      : [bytes];
    let acknowledged = -1, offset = 0;
    for (const input of inputs) {
      await feed(f, input);
      await until(() => f.pending.length === 1, 'next UTF-8 read');
      const part = f.poll(['target', acknowledged]);
      if (part.kind === 'pending') continue;
      assert.equal(part.kind, 'chunk');
      assert.equal(part.done, false);
      assert.equal(part.sequence, acknowledged + 1);
      assert.equal(part.offset, offset);
      const decoded = Buffer.from(part.body, 'utf8').toString('utf8');
      assert.equal(decoded, part.body);
      assert.equal(frame(f, acknowledged), part);
      assert.equal(Buffer.from(frame(f, acknowledged).body, 'utf8').toString('utf8'), decoded);
      received.push(decoded);
      acknowledged = part.sequence;
      offset = part.next;
    }
    await finish(f);
    const last = frame(f, acknowledged);
    assert.equal(last.offset, offset);
    const decoded = Buffer.from(last.body, 'utf8').toString('utf8');
    assert.equal(decoded, last.body);
    received.push(decoded);
    assert.equal(received.join(''), expected);
    assert.equal(last.next, expected.length);
    assert.equal(last.received_bytes, bytes.length);
    close(f, last);
  });
}
