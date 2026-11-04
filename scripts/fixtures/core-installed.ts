import assert from 'node:assert/strict';
import { request, type IncomingMessage } from 'node:http';
import { setImmediate } from 'node:timers/promises';
import { createInstance, type InferenceResponse, type RequestEvent } from '@framerize/core';

const initialResources = process.getActiveResourcesInfo().sort();
let forwarded = 0;
let closed = 0;
const events: RequestEvent[] = [];
const instance = createInstance({ token: 'synthetic-installed-token',
  onRequest(event) { events.push(event); }, backend: {
    async ready() { return true; },
    async forward(input) {
      forwarded += 1;
      assert.equal(input.path, '/v1/responses');
      const chunks: Buffer[] = [];
      for await (const chunk of input.body) chunks.push(chunk);
      assert.equal(Buffer.concat(chunks).toString(), '{"model":"synthetic"}');
      return { status: 200, headers: { 'content-type': 'text/event-stream' },
        body: (async function* () { yield Buffer.from('data: synthetic\n\n'); })() };
    },
    close() { closed += 1; },
  },
});

interface Reply { status: number; body: string; }
function exchange(port: number, path: string, authenticated = true): Promise<Reply> {
  return new Promise((resolve, reject) => {
    const client = request({ host: '127.0.0.1', port, path, agent: false,
      method: path.startsWith('/v1/') ? 'POST' : 'GET',
      headers: authenticated ? { authorization: 'Bearer synthetic-installed-token' } : {},
    }, (response) => collect(response, resolve, reject));
    client.on('error', reject);
    client.end(path.startsWith('/v1/') ? '{"model":"synthetic"}' : undefined);
  });
}

function collect(response: IncomingMessage, resolve: (value: Reply) => void,
  reject: (reason: unknown) => void): void {
  const chunks: Buffer[] = [];
  response.on('data', (chunk: Buffer) => chunks.push(chunk));
  response.on('error', reject);
  response.on('end', () => resolve({ status: response.statusCode!,
    body: Buffer.concat(chunks).toString() }));
}

const { port } = await instance.listen();
try {
  assert.equal((await exchange(port, '/ready', false)).status, 401);
  assert.equal((await exchange(port, '/ready')).status, 200);
  assert.deepEqual(await exchange(port, '/v1/responses'),
    { status: 200, body: 'data: synthetic\n\n' });
  assert.equal(forwarded, 1);
} finally { await instance.close(); }
assert.equal(closed, 1);
assert.equal(events.length, 3);
assert.doesNotMatch(JSON.stringify(events), /synthetic-installed-token/);
const lateReturns = await lateResponse();
await setImmediate();
await setImmediate();
assert.deepEqual(process.getActiveResourcesInfo().sort(), initialResources);
console.log(JSON.stringify({ node: process.version, forwarded, closed, lateReturns,
  retainedResources: process.getActiveResourcesInfo().filter((type) => /TCP|Timeout/.test(type)) }));

async function lateResponse(): Promise<number> {
  let release!: (value: InferenceResponse) => void;
  const pending = new Promise<InferenceResponse>((resolve) => { release = resolve; });
  let returned = 0;
  const late = createInstance({ token: 'synthetic-installed-token', inferenceTimeoutMs: 25,
    backend: { async ready() { return true; }, forward: () => pending, close() {} } });
  try {
    const address = await late.listen();
    assert.equal((await exchange(address.port, '/v1/responses')).status, 504);
    release({ status: 200, headers: {}, body: { [Symbol.asyncIterator]() { return {
      async next() { throw new Error('Discarded body must not be read'); },
      async return() { returned += 1; return { done: true as const, value: undefined }; },
    }; } } });
    await setImmediate();
    assert.equal(returned, 1, 'Installed core must return late response bodies');
  } finally { await late.close(); }
  return returned;
}
