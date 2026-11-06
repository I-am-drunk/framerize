import test from 'node:test';
import assert from 'node:assert/strict';
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

test('ambiguous IPv4 aliases are left for the operating system to resolve', () => {
  for (const host of ['127.1', '2130706433', '0x7f000001', '0177.0.0.1']) {
    const config = normalizeConfig(parsePersisted({ remote: 'http://127.0.0.1:8789', host, port: 0 }), '/synthetic');
    assert.equal(config.host, host);
    assert.equal(isLoopbackHost(config.host), false, host);
  }
});

test('non-loopback aliases and URL syntax do not become loopback hosts', () => {
  for (const host of ['126.1', '0x80000001', '2147483649', '0176.0.0.1', '127.08.0.1',
    '127.0.0.1/path', 'user@127.0.0.1', '127.0.0.1:80', ' 127.0.0.1', '127..0.1']) {
    assert.equal(isLoopbackHost(host), false, host);
  }
});
