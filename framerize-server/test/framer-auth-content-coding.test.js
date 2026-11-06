import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { connect } from 'node:net';
import { gzipSync } from 'node:zlib';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/framer-auth.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { createFramerAuth } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);

async function within(pending, milliseconds, label) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} did not settle`)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function refused(port) {
  const result = await new Promise(resolve => {
    const socket = connect({ host: '127.0.0.1', port });
    socket.once('error', error => { socket.destroy(); resolve(error.code); });
    socket.once('connect', () => { socket.destroy(); resolve('connected'); });
    socket.setTimeout(500, () => { socket.destroy(); resolve('timeout'); });
  });
  assert.equal(result, 'ECONNREFUSED');
}

async function fixture(context, coding) {
  const requests = [];
  const sockets = new Set();
  let auth;
  let port;
  const server = createServer((request, response) => {
    requests.push({ method: request.method, path: request.url, cookie: request.headers.cookie });
    const json = Buffer.from(JSON.stringify({ accessToken: 'synthetic-access-token',
      expiresAt: new Date(Date.now() + 120_000).toISOString() }));
    response.writeHead(200, { 'content-type': 'application/json',
      ...(coding === undefined ? {} : { 'content-encoding': coding }) });
    response.end(coding === 'gzip' ? gzipSync(json) : json);
  });
  server.on('connection', socket => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
  });
  context.after(async () => {
    let authClosed = !auth;
    try {
      if (auth) await within(auth.close(), 1500, 'auth cleanup');
      authClosed = true;
    }
    finally {
      for (const socket of sockets) socket.destroy();
      await within(new Promise(resolve => server.close(resolve)), 1500, 'fixture cleanup');
      if (port !== undefined) await refused(port);
      context.diagnostic(`cleanup: auth closed=${authClosed}; captured fixture port refused connections`);
    }
  });
  await within(new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  }), 1500, 'fixture listen');
  port = server.address().port;
  auth = createFramerAuth({ origin: `http://127.0.0.1:${port}`,
    sessionCookie: 'session=synthetic-session', timeoutMs: 1000 });
  return { auth, requests };
}

for (const coding of [undefined, 'identity', 'IDENTITY', 'iDeNtItY']) {
  test(`authentication accepts ${coding ?? 'absent'} content encoding`, { timeout: 5000 }, async context => {
    const { auth, requests } = await fixture(context, coding);
    assert.equal(await auth.accessToken(), 'synthetic-access-token');
    assert.deepEqual(requests, [{ method: 'GET', path: '/auth/ai/access-token',
      cookie: 'session=synthetic-session' }]);
  });
}

test('authentication still refuses gzip content encoding', { timeout: 5000 }, async context => {
  const { auth, requests } = await fixture(context, 'gzip');
  await assert.rejects(auth.accessToken(), { code: 'auth_unavailable' });
  assert.deepEqual(requests, [{ method: 'GET', path: '/auth/ai/access-token',
    cookie: 'session=synthetic-session' }]);
});
