// Synthetic loopback workload for the completed custom routes through real
// server and Remote listeners. The byte-budget approach follows T3 Code
// apps/server/src/orchestration-v2/ThreadTransportPerformance.test.ts (MIT).
// Timings are local diagnostics, not provider latency or billing evidence.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { performance } from 'node:perf_hooks';
import test from 'node:test';
import { fixture, protocols, reply } from './custom-routes-fixture.mjs';
import { customRequestCases, identify, identityOf } from './custom-request-cases.mjs';

const WARMUP = 2;
const SAMPLES = 16;
const CONCURRENCY = 4;
// Initial measured baseline: at most 605 envelope bytes per request for plain
// text, 460 for the Unicode corpus and 366 for long history. The envelope adds
// fixed fields and at most 300 prompt characters. Copying the request body
// again exceeds this ceiling for every workload, as does copying a 1 KiB prompt.
const MAX_ENVELOPE_OVERHEAD_BYTES = 1024;
// This bounds hangs; it is not a latency regression threshold.
const CASE_DEADLINE_MS = 20_000;

const text = bytes => 'synthetic '.repeat(Math.ceil(bytes / 10)).slice(0, bytes);
const jsonBytes = value => Buffer.byteLength(JSON.stringify(value));
const percentile = (sorted, p) => sorted[Math.ceil(p * sorted.length) - 1];

// Plain text at 1 KiB and 256 KiB, plus the shared request corpus: tools,
// reasoning options, long history, images and documents with UTF-8 text.
function workloads(protocol) {
  const plain = bytes => ({ name: `${bytes} text bytes`, replyBytes: bytes, body(id, index) {
    const content = text(bytes);
    return { ...protocol.body, model: `synthetic-${protocol.name}-${index % 2}`, temperature: 0.25,
      metadata: { case: id }, ...(protocol.name === 'responses'
        ? { input: content } : { messages: [{ role: 'user', content }] }) };
  } });
  return [plain(1024), plain(256 * 1024), ...customRequestCases
    .filter(entry => entry.protocol === protocol.name)
    .map(entry => ({ name: entry.name, replyBytes: 1024, body: id => identify(entry, id) }))];
}

function completed(protocol, bytes) {
  const content = text(bytes);
  return reply(protocol, protocol.name === 'responses'
    ? { output: [{ type: 'message', id: 'synthetic-message', role: 'assistant',
      content: [{ type: 'output_text', text: content, annotations: [] }] }] }
    : { content: [{ type: 'text', text: content }] });
}

// Each measured batch is held at the upstream until all of its requests arrive.
// A server or Remote that serialized requests fails at the fixture's one-second
// header deadline instead of reporting a slower timing.
function batchedUpstream(payload) {
  const held = [];
  return (response, count) => {
    response.writeHead(200, { 'content-type': 'application/json' });
    if (count <= WARMUP) { response.end(payload); return; }
    held.push(response);
    if (held.length === CONCURRENCY) for (const waiting of held.splice(0)) waiting.end(payload);
  };
}

// The fixture's after hook asserts clean shutdown: no forced socket closure,
// closed upstream sockets, closed credentials and released abort listeners.
async function measure(context, protocol, viaRemote, workload) {
  const value = completed(protocol, workload.replyBytes);
  const payload = Buffer.from(JSON.stringify(value));
  const setup = await fixture(context, protocol, viaRemote,
    { value, respond: batchedUpstream(payload) });
  const expected = new Map();
  const timings = [];
  let clientBytes = 0;
  async function send(id, measured) {
    const body = workload.body(`perf-${id}`, id);
    expected.set(`perf-${id}`, body);
    clientBytes += jsonBytes(body);
    const start = performance.now();
    const result = await setup.post(body);
    if (measured) timings.push(performance.now() - start);
    assert.equal(result.status, 200, result.body.toString());
    assert.match(result.headers['content-type'], /^application\/json/);
    assert.deepEqual(result.body, payload);
    assert.deepEqual(JSON.parse(result.body), value);
  }
  for (let id = 0; id < WARMUP; id += 1) await send(id, false);
  const start = performance.now();
  for (let batch = 0; batch < SAMPLES / CONCURRENCY; batch += 1) {
    const outcomes = await Promise.allSettled(Array.from({ length: CONCURRENCY },
      (_, index) => send(WARMUP + batch * CONCURRENCY + index, true)));
    for (const outcome of outcomes) if (outcome.status === 'rejected') throw outcome.reason;
  }
  const durationMs = performance.now() - start;
  const requests = WARMUP + SAMPLES;
  assert.equal(setup.observed.length, requests);
  assert.equal(setup.counts.auth, requests);
  assert.equal(setup.counts.mints, requests);
  assert.equal(setup.serverCalls.length, requests);
  let upstreamBytes = 0;
  let maxOverhead = 0;
  const seen = new Set();
  for (const item of setup.observed) {
    const id = identityOf(item.body.request);
    const sent = expected.get(id);
    assert.ok(sent, `Unknown request identity: ${id}`);
    assert.equal(seen.has(id), false, `Duplicate request identity: ${id}`);
    seen.add(id);
    assert.equal(item.path, protocol.upstream);
    assert.equal(item.headers.authorization, 'Bearer synthetic-upstream');
    assert.deepEqual(item.body.request, { ...sent, stream: false });
    assert.equal(item.body.model, sent.model);
    assert.equal(item.body.projectId, 'synthetic-project');
    assert.equal(item.body.castleToken, 'synthetic-castle');
    const size = item.requestBytes;
    upstreamBytes += size;
    maxOverhead = Math.max(maxOverhead, size - jsonBytes(sent));
  }
  assert.equal(expected.size, requests);
  assert.ok(maxOverhead <= MAX_ENVELOPE_OVERHEAD_BYTES,
    `${protocol.name} envelope grew to ${maxOverhead} bytes per request`);
  const sorted = timings.toSorted((a, b) => a - b);
  return { protocol: protocol.name, route: protocol.path, upstream: protocol.upstream, viaRemote,
    workload: workload.name, requests, measured: SAMPLES, concurrency: CONCURRENCY,
    clientRequestBytes: clientBytes, upstreamRequestBytes: upstreamBytes,
    maxEnvelopeOverheadBytes: maxOverhead, responseBytesPerRequest: payload.length,
    durationMs: round(durationMs), p50Ms: round(percentile(sorted, 0.5)),
    p95Ms: round(percentile(sorted, 0.95)), maxMs: round(sorted.at(-1)),
    requestsPerSecond: round(SAMPLES * 1000 / durationMs) };
}

const round = value => Math.round(value * 100) / 100;

// GitHub checks out a pull request's merge commit, so HEAD is the tested tree.
function source() {
  try {
    const git = args => execFileSync('git', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
    return { sha: git(['rev-parse', 'HEAD']).trim(),
      clean: git(['status', '--porcelain', '--untracked-files=normal']).trim() === '' };
  } catch { return { sha: 'unknown', clean: false }; }
}

const cases = [];
for (const protocol of protocols) for (const viaRemote of [false, true]) {
  for (const workload of workloads(protocol)) {
    test(`synthetic ${protocol.name} workload, remote=${viaRemote}: ${workload.name}`,
      { timeout: CASE_DEADLINE_MS },
      async context => { cases.push(await measure(context, protocol, viaRemote, workload)); });
  }
}

test.after(() => {
  const file = process.env.FRAMERIZE_PERFORMANCE_REPORT;
  if (!file) return;
  writeFileSync(file, JSON.stringify({ schema: 1, source: source(), node: process.version,
    platform: process.platform, arch: process.arch,
    expectedCases: protocols.reduce((count, protocol) => count + workloads(protocol).length * 2, 0),
    scope: 'Synthetic loopback measurement through real server and Remote listeners. '
      + 'Not provider latency, live inference or billing evidence. Timings are not gated.',
    gated: ['status and exact reply bytes', 'upstream route', 'model and options', 'stream:false',
      'one upstream request, access token and Castle mint per client request',
      `envelope overhead <= ${MAX_ENVELOPE_OVERHEAD_BYTES} bytes`, `concurrency ${CONCURRENCY}`,
      `case deadline ${CASE_DEADLINE_MS} ms`, 'clean shutdown'],
    cases }, null, 2) + '\n');
});
