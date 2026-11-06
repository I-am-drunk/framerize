import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer, connect } from 'node:net';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { parsePersisted } from './config.ts'; export { normalizeConfig } from './configStore.ts'; export { isLoopbackHost } from './hostPolicy.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
const { parsePersisted, normalizeConfig, isLoopbackHost } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
console.log(JSON.stringify({ runtime: process.version, bundle: createHash('sha256')
  .update(compiled.outputFiles[0].contents).digest('hex') }));

async function refused(port) {
  await assert.rejects(new Promise((resolve, reject) => {
    const socket = connect({ host: '127.0.0.1', port });
    socket.once('error', reject);
    socket.once('connect', () => { socket.destroy(); resolve(); });
    socket.setTimeout(1000, () => socket.destroy(new Error('fixture deadline exceeded')));
  }), { code: 'ECONNREFUSED' });
}

test('accepted numeric IPv4 aliases report the loopback address they bind', { timeout: 5000 }, async context => {
  const results = [];
  for (const host of ['127.1', '2130706433', '0x7f000001', '0177.0.0.1']) {
    const server = createServer();
    context.after(() => new Promise(resolve => server.close(resolve)));
    const config = normalizeConfig(parsePersisted({ remote: 'http://127.0.0.1:8789', host, port: 0 }), '/synthetic');
    await new Promise((resolve, reject) => {
      server.once('error', reject);
      server.listen({ host: config.host, port: config.port }, resolve);
    });
    const address = server.address();
    assert.equal(address.address, '127.0.0.1', host);
    results.push({ host, local: isLoopbackHost(config.host) });
    await new Promise(resolve => server.close(resolve));
    await refused(address.port);
  }
  context.diagnostic(JSON.stringify(results));
  assert.deepEqual(results.map(result => result.local), [true, true, true, true]);
});

test('non-loopback aliases and URL syntax do not become loopback hosts', () => {
  for (const host of ['126.1', '0x80000001', '2147483649', '0176.0.0.1', '127.08.0.1',
    '127.0.0.1/path', 'user@127.0.0.1', '127.0.0.1:80', ' 127.0.0.1', '127..0.1']) {
    assert.equal(isLoopbackHost(host), false, host);
  }
});
