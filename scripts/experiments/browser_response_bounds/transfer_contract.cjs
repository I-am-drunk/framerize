'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { Script } = require('node:vm');

const inputPath = join(__dirname, 'transfer.js');
const program = new Script(`(${readFileSync(inputPath, 'utf8')})`, {
  filename: inputPath,
});
const sizes = [32768, 131072];
const jobError = 'completed successful fixture job required';
const inputError = 'invalid fixture transfer offset or size';

function fixture(body = 'fixture body') {
  const timer = { label: 'target timer' };
  const job = { done: true, value: { status: 200, usage: { fixture: true }, body }, timer };
  const window = { __aiJobs: { target: job } };
  const cleared = [];
  const take = program.runInNewContext({
    window,
    clearTimeout: handle => cleared.push(handle),
  });
  return { take, job, window, cleared, timer };
}

function reject(f, args, message, label) {
  const jobs = f.window.__aiJobs, target = jobs?.target;
  const offset = f.job.transferOffset, timer = f.job.timer;
  const cleared = f.cleared.slice();
  assert.throws(() => f.take(args), { name: 'Error', message }, label);
  assert.equal(f.window.__aiJobs, jobs, label);
  assert.equal(jobs?.target, target, label);
  assert.equal(f.job.transferOffset, offset, label);
  assert.equal(f.job.timer, timer, label);
  assert.deepEqual(f.cleared, cleared, label);
}

function drain(f, limit) {
  const body = f.job.value.body, usage = f.job.value.usage;
  const chunks = [];
  let offset = 0;
  for (let step = 0; step < Math.ceil(body.length / limit) + 2; step++) {
    const chunk = f.take(['target', offset, limit]);
    assert.equal(chunk.status, 200);
    assert.equal(chunk.usage, usage);
    assert.equal(chunk.offset, offset);
    assert.equal(chunk.next, offset + chunk.body.length);
    assert.equal(chunk.done, chunk.next === body.length);
    assert.ok(chunk.body.length <= limit);
    const decoded = Buffer.from(chunk.body, 'utf8').toString('utf8');
    assert.equal(decoded, chunk.body, 'each chunk survives UTF-8 transfer');
    chunks.push(decoded);
    if (chunk.done) {
      assert.equal(Object.hasOwn(f.window.__aiJobs, 'target'), false);
      assert.deepEqual(f.cleared, [f.timer]);
      assert.equal(chunks.join(''), body);
      return chunks;
    }
    assert.ok(chunk.next > offset, 'each non-final chunk makes progress');
    assert.equal(f.window.__aiJobs.target, f.job);
    assert.equal(f.job.transferOffset, chunk.next);
    assert.deepEqual(f.cleared, []);
    offset = chunk.next;
  }
  assert.fail('transfer exceeded the bounded number of chunk calls');
}

for (const limit of sizes) {
  test(`ASCII transfer at ${limit} units drains and finalizes once`, () => {
    const f = fixture('a'.repeat(limit) + 'b'.repeat(limit) + 'tail');
    const chunks = drain(f, limit);
    assert.deepEqual(chunks.map(chunk => chunk.length), [limit, limit, 4]);
    reject(f, ['target', 0, limit], jobError, 'completed job is removed');
  });

  test(`non-BMP text crosses successive ${limit}-unit boundaries intact`, () => {
    const rocket = '\u{1F680}', flask = '\u{1F9EA}';
    const body = 'x'.repeat(limit - 1) + rocket + 'y'.repeat(limit - 3) + flask + 'tail';
    const chunks = drain(fixture(body), limit);
    assert.equal(chunks.length, 3);
    assert.ok(chunks[1].startsWith(rocket));
    assert.ok(chunks[2].startsWith(flask));
  });
}

test('empty body emits one final chunk and is removed only once', () => {
  const f = fixture('');
  assert.deepEqual(drain(f, sizes[0]), ['']);
  reject(f, ['target', 0, sizes[0]], jobError, 'empty job is removed');
});

test('invalid offset types and ranges preserve the job and timer', () => {
  const offsets = [undefined, null, '0', false, true, 0.5, -1, NaN,
    Infinity, -Infinity, Number.MAX_SAFE_INTEGER + 1, 0n, Symbol('offset')];
  for (const offset of offsets) {
    reject(fixture(), ['target', offset, sizes[0]], inputError, String(offset));
  }
});

test('unsupported chunk sizes preserve the job and timer', () => {
  const limits = [undefined, null, false, 0, 1, 32767, 32769, 131071,
    131073, '32768', NaN, Infinity];
  for (const limit of limits) {
    reject(fixture(), ['target', 0, limit], inputError, String(limit));
  }
});

test('repeated and out-of-order offsets cannot advance or finalize a job', () => {
  const limit = sizes[0], f = fixture('x'.repeat(2 * limit + 1));
  reject(f, ['target', 1, limit], inputError, 'nonzero first offset');
  reject(f, ['target', limit, limit], inputError, 'skipped first chunk');
  const first = f.take(['target', 0, limit]);
  assert.equal(first.next, limit);
  assert.equal(first.done, false);
  for (const offset of [0, 1, limit - 1, limit + 1, 2 * limit, Number.MAX_SAFE_INTEGER]) {
    reject(f, ['target', offset, limit], inputError, `offset ${offset}`);
  }
  const second = f.take(['target', first.next, limit]);
  assert.equal(second.next, 2 * limit);
  assert.equal(second.done, false);
  const final = f.take(['target', second.next, limit]);
  assert.equal(final.body, 'x');
  assert.equal(final.done, true);
  assert.equal(Object.hasOwn(f.window.__aiJobs, 'target'), false);
  assert.deepEqual(f.cleared, [f.timer]);
});

test('missing, incomplete and unsuccessful jobs refuse without cleanup', () => {
  const changes = [
    ['missing map', f => { delete f.window.__aiJobs; }],
    ['null map', f => { f.window.__aiJobs = null; }],
    ['missing job', f => { delete f.window.__aiJobs.target; }],
    ['null job', f => { f.window.__aiJobs.target = null; }],
    ['incomplete job', f => { f.job.done = false; }],
    ['missing done flag', f => { delete f.job.done; }],
    ['missing value', f => { delete f.job.value; }],
    ['null value', f => { f.job.value = null; }],
    ['unsuccessful status', f => { f.job.value.status = 503; }],
    ['string status', f => { f.job.value.status = '200'; }],
    ['missing status', f => { delete f.job.value.status; }],
  ];
  for (const [label, change] of changes) {
    const f = fixture();
    change(f);
    reject(f, ['target', 0, sizes[0]], jobError, label);
  }
});

test('an unrelated job and its timer survive refusal and target finalization', () => {
  const f = fixture('x'.repeat(sizes[0] + 1));
  const otherTimer = { label: 'unrelated timer' };
  const other = { done: false, timer: otherTimer, transferOffset: 3,
    value: { status: 200, usage: 17, body: 'unrelated body' } };
  const original = { ...other, value: { ...other.value } };
  f.window.__aiJobs.other = other;
  reject(f, ['target', 1, sizes[0]], inputError, 'unrelated job during refusal');
  drain(f, sizes[0]);
  assert.deepEqual(Object.keys(f.window.__aiJobs), ['other']);
  assert.equal(f.window.__aiJobs.other, other);
  assert.equal(other.timer, otherTimer);
  assert.deepEqual(other, original);
  assert.ok(!f.cleared.includes(otherTimer));
});
