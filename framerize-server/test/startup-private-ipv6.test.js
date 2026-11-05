import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/startup-access.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const bundleSha256 = createHash('sha256').update(bytes).digest('hex');
const { resolveHeadlessConnectionString, buildPairingUrl } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
const ipv6 = (address, internal = false) => ({ address, internal, family: 'IPv6', scopeid: 0,
  netmask: 'ffff:ffff:ffff:ffff::', mac: '00:00:00:00:00:00', cidr: `${address}/64` });
const ula = ipv6('fd12:3456:789a::7');
const external = ipv6('2001:db8::7');
const linkLocal = ipv6('fe80::7');
const ipv4 = { address: '192.168.1.7', internal: false, family: 'IPv4',
  netmask: '255.255.255.0', mac: '00:00:00:00:00:00', cidr: '192.168.1.7/24' };
const cases = [
  { name: 'IPv6 private wildcard advertises an available ULA', host: '::',
    interfaces: { eth0: [ula] }, expected: 'http://[fd12:3456:789a::7]:8789' },
  { name: 'private IPv6 selection excludes public, link-local and virtual candidates', host: '[::]',
    interfaces: { docker0: [ipv6('fd12:3456:789a::d')], eth0: [external, linkLocal, ula] },
    expected: 'http://[fd12:3456:789a::7]:8789' },
  { name: 'IPv4-only bind never selects a ULA', host: '0.0.0.0',
    interfaces: { eth0: [ula] }, expected: 'http://localhost:8789' },
  { name: 'eligible IPv4 remains preferred over ULA', host: '::',
    interfaces: { eth0: [ula, ipv4] }, expected: 'http://192.168.1.7:8789' },
  { name: 'private mode retains fallback with no eligible address', host: '::',
    interfaces: { eth0: [external, linkLocal], lo: [ipv6('::1', true)] }, expected: 'http://localhost:8789' },
  { name: 'nonprivate mode keeps its existing IPv6 preference', host: '::', privateNetworkOnly: false,
    interfaces: { eth0: [external, ula] }, expected: 'http://[2001:db8::7]:8789' },
];

for (const fixture of cases) {
  test(fixture.name, { timeout: 1000 }, context => {
    const original = JSON.stringify(fixture.interfaces);
    const origin = resolveHeadlessConnectionString(fixture.host, 8789, {
      interfaces: fixture.interfaces, privateNetworkOnly: fixture.privateNetworkOnly ?? true,
    });
    const pairing = new URL(buildPairingUrl(origin));
    context.diagnostic(JSON.stringify({ host: fixture.host, origin, pairing: pairing.href,
      bundleSha256, syntheticInterfaces: true }));
    assert.equal(JSON.stringify(fixture.interfaces), original);
    assert.equal(pairing.origin, origin);
    assert.equal(pairing.pathname, '/pair');
    assert.equal(pairing.search, '');
    assert.equal(pairing.hash, '');
    assert.equal(origin, fixture.expected);
  });
}
