import assert from 'node:assert/strict';
import { execFile, spawnSync } from 'node:child_process';
import { once, getEventListeners } from 'node:events';
import { readFileSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import test from 'node:test';
import { runInferenceCheck } from '../scripts/check-inference.js';
import { inspectCompletion, inspectJsonCompletion } from '../scripts/validation/completion.js';
import { checkInference } from '../scripts/validation/inference.js';
import { privateEnvironment } from '../scripts/core-package-check.js';
import { assertTransportClosed, transportResources } from '../scripts/fixtures/transport-scenario.js';

const marker = 'synthetic-private-marker';
const event = (delta: object, finish: string | null = null) => `data: ${JSON.stringify({
  choices: [{ index: 0, delta, finish_reason: finish }],
})}\n\n`;
const stream = event({ content: marker }) + event({}, 'stop') + 'data: [DONE]\n\n';
const baseArgs = ['--endpoint', 'http://127.0.0.1:1', '--model', 'fixture', '--token-env', 'TOKEN'];
const entrypoint = fileURLToPath(new URL('../scripts/check-inference.js', import.meta.url));

const customProtocols = ['responses', 'messages'] as const;
for (const protocol of customProtocols) {
  test(`custom ${protocol} check sends one completed-JSON request`, async () => {
    let body = '';
    await fixture((request, response) => {
      assert.equal(request.headers.authorization, `Bearer ${marker}`);
      assert.equal(request.headers.accept, 'application/json');
      request.on('data', chunk => { body += String(chunk); });
      request.on('end', () => {
        response.writeHead(200, { 'content-type': 'application/json; charset=utf-8' });
        response.end(JSON.stringify(jsonReply(protocol)));
      });
    }, async (endpoint, calls) => {
      const result = await runInferenceCheck(['--endpoint', endpoint, '--model', 'fixture',
        '--token-env', 'TOKEN', '--send-one-request', '--protocol', protocol], { TOKEN: marker });
      assert.equal(result.ok, true);
      assert.equal(result.requestAttempts, 1);
      if (result.ok) assert.equal(result.contentBytes, Buffer.byteLength(marker));
      assert.deepEqual(calls, [`POST ${protocol === 'responses'
        ? '/openai/v1/responses' : '/anthropic/v1/messages'}`]);
      assert.deepEqual(JSON.parse(body), { model: 'fixture', stream: false,
        ...(protocol === 'responses' ? { input: 'Reply with exactly OK.', max_output_tokens: 64 }
          : { messages: [{ role: 'user', content: 'Reply with exactly OK.' }], max_tokens: 64 }) });
      assert.doesNotMatch(JSON.stringify(result), new RegExp(marker));
    });
  });
}
for (const protocol of customProtocols) {
  test(`custom ${protocol} text validation rejects non-success without exposing content`, () => {
    const invalid = protocol === 'responses' ? [
      { status: 'incomplete', incomplete_details: { reason: 'max_output_tokens' } },
      { status: 'queued' }, { output: [] },
      { output: [{ type: 'function_call', call_id: 'call', name: 'tool', arguments: '{}' }] },
      { output: [{ id: 'message', type: 'message', role: 'assistant',
        content: [{ type: 'refusal', refusal: marker }] }] },
      { output: [{ id: 'message', type: 'message', role: 'assistant', status: 'failed',
        content: [{ type: 'output_text', text: marker }] }] },
    ] : [
      { stop_reason: 'max_tokens' }, { stop_reason: 'refusal' }, { content: [] },
      { stop_reason: 'tool_use', content: [{ type: 'tool_use', id: 'call', name: 'tool', input: {} }] },
      { stop_reason: 'pause_turn' }, { content: [{ type: 'text', text: '   ' }] },
    ];
    for (const changes of invalid) {
      assert.throws(() => inspectJsonCompletion(protocol, Buffer.from(JSON.stringify(jsonReply(protocol, changes)))));
    }
    for (const bytes of [Buffer.from(marker), Buffer.from([0xff]), Buffer.from('{}')]) {
      assert.throws(() => inspectJsonCompletion(protocol, bytes));
    }
    assert.deepEqual(inspectJsonCompletion(protocol, Buffer.from(JSON.stringify(jsonReply(protocol)))),
      { contentBytes: Buffer.byteLength(marker), textBlocks: 1 });
  });
}
for (const protocol of customProtocols) for (const mode of ['malformed', 'limited', 'compressed',
  'oversized', 'wrong-type', 'truncated', 'redirect', 'refused'] as const) {
  test(`custom ${protocol} check rejects ${mode} with one request`, async () => {
    await fixture((_request, response) => {
      response.writeHead(mode === 'refused' ? 429 : mode === 'redirect' ? 302 : 200, {
        'content-type': mode === 'wrong-type' ? 'text/event-stream' : 'application/json',
        ...(mode === 'compressed' ? { 'content-encoding': 'gzip' } : {}),
        ...(mode === 'redirect' ? { location: '/must-not-follow' } : {}),
        ...(mode === 'truncated' ? { 'content-length': 9999 } : {}),
      });
      if (mode === 'truncated') {
        response.write('{'); setImmediate(() => response.destroy()); return;
      }
      response.end(mode === 'oversized' ? Buffer.alloc(65537, 97)
        : mode === 'malformed' || mode === 'refused' ? marker
          : JSON.stringify(jsonReply(protocol, mode !== 'limited' ? {}
            : protocol === 'responses' ? { status: 'incomplete', incomplete_details: { reason: 'max_output_tokens' } }
              : { stop_reason: 'max_tokens' })));
    }, async (endpoint, calls) => {
      const result = await checkInference({ endpoint, token: marker, model: 'fixture', protocol });
      assert.equal(result.ok, false);
      if (!result.ok) assert.equal(result.code, mode === 'refused' || mode === 'redirect' ? 'http_refused'
        : mode === 'truncated' ? 'transport' : mode === 'oversized' ? 'response_too_large'
          : mode === 'compressed' ? 'unsupported_encoding' : 'invalid_response');
      assert.equal(calls.length, 1);
      assert.doesNotMatch(JSON.stringify(result), new RegExp(marker));
    });
  });
}
for (const protocol of customProtocols) {
  test(`custom ${protocol} CLI accepts a private credential reference`, async () => {
    const root = mkdtempSync(join(tmpdir(), 'custom inference cli '));
    try {
      await fixture((_request, response) => {
        response.writeHead(200, { 'content-type': 'application/json' });
        response.end(JSON.stringify(jsonReply(protocol)));
      }, async (endpoint, calls) => {
        const { stdout, stderr } = await promisify(execFile)(process.execPath, [entrypoint,
          '--endpoint', endpoint, '--model', 'fixture', '--protocol', protocol,
          '--token-env', 'TOKEN', '--send-one-request'], { cwd: root,
          env: { ...privateEnvironment(root), TOKEN: marker }, timeout: 5000, maxBuffer: 65536 });
        assert.equal(JSON.parse(stdout).ok, true);
        assert.equal(JSON.parse(stdout).protocol, protocol);
        assert.equal(JSON.parse(stdout).requestAttempts, 1);
        assert.equal(stderr, '');
        assert.equal(calls.length, 1);
        assert.doesNotMatch(stdout, new RegExp(marker));
      });
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
}
for (const protocol of customProtocols) for (const mode of ['abort', 'timeout'] as const) {
  test(`custom ${protocol} ${mode} closes transport before fixture shutdown`, async () => {
    const controller = new AbortController();
    await fixture((_request, response) => {
      response.writeHead(200, { 'content-type': 'application/json' }); response.write('{');
      if (mode === 'abort') setImmediate(() => controller.abort(new Error(marker)));
    }, async (endpoint, calls) => {
      const result = await checkInference({ endpoint, token: marker, model: 'fixture', protocol,
        timeoutMs: mode === 'timeout' ? 75 : 3000, signal: controller.signal });
      assert.equal(result.ok, false);
      if (!result.ok) assert.equal(result.code, mode === 'abort' ? 'aborted' : 'timeout');
      assert.equal(calls.length, 1);
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
      assert.doesNotMatch(JSON.stringify(result), new RegExp(marker));
    });
  });
}
function jsonReply(protocol: typeof customProtocols[number], changes: object = {}) {
  return protocol === 'responses' ? { id: 'reply-fixture', object: 'response', model: 'fixture',
    created_at: 1, status: 'completed', output: [{ id: 'message-fixture', type: 'message',
      role: 'assistant', content: [{ type: 'output_text', text: marker }] }], ...changes }
    : { id: 'reply-fixture', type: 'message', model: 'fixture', role: 'assistant',
      stop_reason: 'end_turn', content: [{ type: 'text', text: marker }], ...changes };
}

test('custom check rejects unknown protocols before reading credentials', async () => {
  let reads = 0;
  const env = new Proxy({}, { get() { reads += 1; throw new Error(marker); } });
  for (const protocol of [marker, '/openai/v1/responses', 'Responses', '']) {
    const result = await runInferenceCheck([...baseArgs, '--send-one-request', '--protocol', protocol], env);
    assert.equal(result.ok, false);
    if (!result.ok) assert.equal(result.code, 'configuration');
    assert.equal(result.requestAttempts, 0);
    assert.doesNotMatch(JSON.stringify(result), new RegExp(marker));
  }
  assert.equal(reads, 0);
});

test('configuration failures do not access credentials or disclose input', async () => {
  let reads = 0;
  const env = new Proxy({}, { get() { reads += 1; throw new Error(marker); } });
  const invalid = [baseArgs, [...baseArgs, '--send-one-request', '--token-file', marker],
    [...baseArgs, '--send-one-request', '--token', marker],
    [...baseArgs, '--send-one-request', '--timeout-ms', '-1'],
    [...baseArgs, '--send-one-request', '--timeout-ms', '60001'],
    [...baseArgs, '--send-one-request', '--timeout-ms', '1e3'],
    [...baseArgs, '--send-one-request', '--model', marker],
    ['--endpoint', `https://${marker}@example.com`, '--model', 'fixture',
      '--token-env', 'TOKEN', '--send-one-request']];
  for (const argv of invalid) {
    const result = await runInferenceCheck(argv, env);
    assert.equal(result.ok, false);
    if (!result.ok) assert.equal(result.code, 'configuration');
    assert.equal(result.requestAttempts, 0);
    assert.doesNotMatch(JSON.stringify(result), new RegExp(marker));
  }
  assert.equal(reads, 0);
});

test('CI invokes offline tests, never the live validation command', () => {
  const source = readFileSync(new URL('../../.github/workflows/core-ci.yml', import.meta.url), 'utf8');
  const validate = (workflow: string) => {
    assert.match(workflow, /run: npm run test:inference\s*\r?\n/);
    assert.doesNotMatch(workflow, /check:inference|check-inference\.js|--send-one-request/);
    for (const path of ['scripts/check-inference.ts', 'scripts/validation/**', 'test/inference-check.test.ts']) {
      assert.equal(workflow.split(`'${path}'`).length - 1, 2);
    }
  };
  validate(source);
  assert.throws(() => validate(source.replace('test:inference', 'check:inference')));
  assert.throws(() => validate(source.replace('scripts/validation/**', 'missing')));
  const manifest = JSON.parse(readFileSync(new URL('../../package.json', import.meta.url), 'utf8'));
  assert.equal(manifest.scripts['test:inference'], 'node --test .tools/test/inference-check.test.js');
});

test('invalid request values fail before attempting transport', async () => {
  const valid = { endpoint: 'http://127.0.0.1:1', token: marker, model: 'fixture' };
  const invalid = [{ ...valid, token: `${marker}\n` }, { ...valid, model: '' },
    { ...valid, model: marker.repeat(20) }, { ...valid, endpoint: 'http://example.com' },
    { ...valid, endpoint: `https://example.com/${marker}` }, { ...valid, timeoutMs: Infinity },
    { ...valid, timeoutMs: 0 }, { ...valid, token: '' },
    new Proxy(valid, { get() { throw new Error(marker); } })];
  for (const input of invalid) {
    const result = await checkInference(input);
    assert.equal(result.ok, false);
    if (!result.ok) assert.equal(result.code, 'configuration');
    assert.equal(result.requestAttempts, 0);
    assert.doesNotMatch(JSON.stringify(result), new RegExp(marker));
  }
});

test('pre-abort skips credential access and throwing credential getters are sanitized', async () => {
  let reads = 0;
  const env = new Proxy({}, { get() { reads += 1; throw new Error(marker); } });
  const controller = new AbortController();
  controller.abort(new Error(marker));
  for (const signal of [controller.signal, undefined]) {
    const result = await runInferenceCheck([...baseArgs, '--send-one-request'], env, signal);
    assert.equal(result.ok, false);
    if (!result.ok) assert.equal(result.code, signal ? 'aborted' : 'credential');
    assert.equal(result.requestAttempts, 0);
    assert.doesNotMatch(JSON.stringify(result), new RegExp(marker));
    assert.equal(reads, signal ? 0 : 1);
  }
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

async function fixture(reply: (request: IncomingMessage, response: ServerResponse) => void,
  run: (endpoint: string, calls: string[]) => Promise<void>) {
  const baseline = transportResources();
  const calls: string[] = [];
  const server = createServer((request, response) => {
    calls.push(`${request.method} ${request.url}`);
    reply(request, response);
  });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const listeningResources = transportResources();
  try {
    await run(`http://127.0.0.1:${address.port}`, calls);
    await assertTransportClosed(listeningResources);
    const connections = await new Promise<number>((resolve, reject) =>
      server.getConnections((error, count) => error ? reject(error) : resolve(count)));
    assert.equal(connections, 0, 'Client connections must close before fixture teardown');
  }
  finally {
    server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
  }
  await assertTransportClosed(baseline);
}

test('one POST completes text inference without printing secrets or response text', async () => {
  let payload = '';
  let authorization: string | undefined;
  await fixture((request, response) => {
    authorization = request.headers.authorization;
    request.on('data', chunk => { payload += String(chunk); });
    request.on('end', () => {
      response.writeHead(200, { 'content-type': 'text/event-stream; charset=utf-8' });
      response.write(stream.slice(0, 13)); response.end(stream.slice(13));
    });
  }, async (endpoint, calls) => {
    const result = await checkInference({ endpoint, token: marker, model: marker });
    assert.equal(result.ok, true);
    assert.equal(result.requestAttempts, 1);
    assert.deepEqual(calls, ['POST /v1/chat/completions']);
    assert.equal(authorization, `Bearer ${marker}`);
    assert.deepEqual(JSON.parse(payload), { model: marker, stream: true,
      messages: [{ role: 'user', content: 'Reply with exactly OK.' }] });
    assert.doesNotMatch(JSON.stringify(result), new RegExp(marker));
  });
});

test('the CLI uses private credential references from a different working directory', async () => {
  const root = mkdtempSync(join(tmpdir(), 'inference cli '));
  const tokenFile = join(root, 'token');
  writeFileSync(tokenFile, marker + '\n', { mode: 0o600 });
  try {
    await fixture((request, response) => {
      assert.equal(request.headers.authorization, `Bearer ${marker}`);
      response.writeHead(200, { 'content-type': 'text/event-stream' }); response.end(stream);
    }, async (endpoint, calls) => {
      for (const reference of [['--token-file', tokenFile], ['--token-env', 'FIXTURE_TOKEN']]) {
        const { stdout, stderr } = await promisify(execFile)(process.execPath, [entrypoint,
          '--endpoint', endpoint, '--model', 'fixture', ...reference, '--send-one-request'],
        { cwd: root, env: { ...privateEnvironment(root), FIXTURE_TOKEN: marker },
          timeout: 10000, maxBuffer: 65536 });
        assert.equal(JSON.parse(stdout).ok, true);
        assert.equal(JSON.parse(stdout).requestAttempts, 1);
        assert.equal(stderr, '');
        assert.doesNotMatch(stdout, new RegExp(marker));
      }
      assert.equal(calls.length, 2);
    });
    assert.equal(readFileSync(tokenFile, 'utf8'), marker + '\n');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('CLI help and rejected invocation exit without disclosing arguments', () => {
  const root = mkdtempSync(join(tmpdir(), 'inference cli failure '));
  try {
    for (const argv of [['--help'], [...baseArgs], [...baseArgs, '--send-one-request'],
      ['--token', marker], [...baseArgs, '--send-one-request', '--token-file', marker]]) {
      const result = spawnSync(process.execPath, [entrypoint, ...argv], { cwd: root,
        env: privateEnvironment(root), encoding: 'utf8', timeout: 5000, maxBuffer: 65536 });
      assert.equal(result.error, undefined);
      assert.equal(result.signal, null);
      assert.equal(result.status, argv[0] === '--help' ? 0 : 1);
      assert.equal(result.stderr, '');
      assert.doesNotMatch(result.stdout, new RegExp(marker));
      if (argv[0] === '--help') assert.match(result.stdout, /--send-one-request/);
      else {
        assert.equal(JSON.parse(result.stdout).ok, false);
        assert.equal(JSON.parse(result.stdout).requestAttempts, 0);
      }
    }
  } finally { rmSync(root, { recursive: true, force: true }); }
});

for (const status of [301, 401, 403, 429, 503]) {
  test(`HTTP ${status} is reported without retries, redirects or response disclosure`, async () => {
    await fixture((_request, response) => {
      response.writeHead(status, { location: '/must-not-follow' }); response.end(marker);
    }, async (endpoint, calls) => {
      const result = await checkInference({ endpoint, token: marker, model: 'fixture' });
      assert.equal(result.ok, false);
      if (!result.ok) assert.equal(result.code, 'http_refused');
      assert.equal(result.status, status);
      assert.equal(calls.length, 1);
      assert.doesNotMatch(JSON.stringify(result), new RegExp(marker));
    });
  });
}

test('SSE parser accepts LF, CRLF, CR, comments and usage after completion', () => {
  const input = ': keepalive\n\n' + stream.replace('data: [DONE]', 'data: {"choices":[],"usage":{}}\n\ndata: [DONE]');
  for (const newline of ['\n', '\r\n', '\r']) {
    assert.deepEqual(inspectCompletion(Buffer.from(input.replaceAll('\n', newline))),
      { events: 3, contentBytes: Buffer.byteLength(marker) });
  }
});

test('incomplete, refused, malformed and substituted completions cannot pass', () => {
  const invalid = ['data: [DONE]\n\n', stream.trimEnd(),
    stream.replace('data: [DONE]\n\n', ''), stream.replace(event({}, 'stop'), ''),
    stream.replace('"stop"', '"length"'), stream.replace('"index":0', '"index":1'),
    stream + event({ content: 'extra' }), 'data: {"error":"private"}\n\n' + stream,
    'event: error\n' + stream, stream.replace(marker, '  '),
    event({ tool_calls: [] }) + stream, 'data: not-json\n\n' + stream];
  for (const value of invalid) assert.throws(() => inspectCompletion(Buffer.from(value)));
  assert.throws(() => inspectCompletion(Buffer.concat([Buffer.from([0xff]), Buffer.from(stream)])));
});

for (const mode of ['oversized', 'compressed', 'wrong-type', 'malformed'] as const) {
  test(`rejects ${mode} responses without further requests`, async () => {
    await fixture((_request, response) => {
      response.setHeader('content-type', mode === 'wrong-type' ? 'application/json' : 'text/event-stream');
      if (mode === 'compressed') response.setHeader('content-encoding', 'gzip');
      response.end(mode === 'oversized' ? Buffer.alloc(65537, 97)
        : mode === 'malformed' ? `data: ${marker}\n\n` : stream);
    }, async (endpoint, calls) => {
      const result = await checkInference({ endpoint, token: marker, model: 'fixture' });
      assert.equal(result.ok, false);
      if (!result.ok) assert.equal(result.code, mode === 'oversized' ? 'response_too_large'
        : mode === 'compressed' ? 'unsupported_encoding' : 'invalid_stream');
      assert.equal(calls.length, 1);
    });
  });
}

test('an exact 64 KiB stream is accepted and a truncated transport is rejected', async () => {
  for (const truncated of [false, true]) {
    await fixture((_request, response) => {
      response.writeHead(200, { 'content-type': 'text/event-stream' });
      if (truncated) {
        response.write(`data: ${marker}`);
        setImmediate(() => response.destroy());
      } else response.end(':' + ' '.repeat(65536 - Buffer.byteLength(stream) - 3) + '\n\n' + stream);
    }, async (endpoint, calls) => {
      const result = await checkInference({ endpoint, token: marker, model: 'fixture' });
      assert.equal(result.ok, !truncated);
      if (result.ok) assert.equal(result.responseBytes, 65536);
      else assert.equal(result.code, 'transport');
      assert.equal(calls.length, 1);
      assert.doesNotMatch(JSON.stringify(result), new RegExp(marker));
    });
  }
});

for (const mode of ['abort', 'timeout', 'headers-abort', 'headers-timeout'] as const) {
  test(`${mode} after server entry cleans up without retrying`, async () => {
    const controller = new AbortController();
    await fixture((_request, response) => {
      if (!mode.startsWith('headers-')) {
        response.writeHead(200, { 'content-type': 'text/event-stream' });
        response.write('data: ');
      }
      if (mode.endsWith('abort')) setImmediate(() => controller.abort(new Error(marker)));
    }, async (endpoint, calls) => {
      const result = await checkInference({ endpoint, token: marker, model: 'fixture',
        timeoutMs: mode.endsWith('timeout') ? 300 : 3000, signal: controller.signal });
      assert.equal(result.ok, false);
      if (!result.ok) assert.equal(result.code, mode.endsWith('abort') ? 'aborted' : 'timeout');
      assert.equal(calls.length, 1);
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
      assert.doesNotMatch(JSON.stringify(result), new RegExp(marker));
    });
  });
}
