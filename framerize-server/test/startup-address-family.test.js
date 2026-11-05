import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/startup-access.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const bytes = compiled.outputFiles[0].contents;
const sourceDigest = createHash('sha256').update(bytes).digest('hex');
const { resolveHeadlessConnectionString, buildPairingUrl } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
const ipv6 = { address: '2001:db8::7', netmask: 'ffff:ffff:ffff:ffff::',
  family: 'IPv6', mac: '00:00:00:00:00:00', internal: false, cidr: '2001:db8::7/64', scopeid: 0 };
const ipv4 = { address: '192.168.1.7', netmask: '255.255.255.0',
  family: 'IPv4', mac: '00:00:00:00:00:00', internal: false, cidr: '192.168.1.7/24' };
const cases = [
  { name: 'IPv4 wildcard never advertises an IPv6-only interface', host: '0.0.0.0',
    interfaces: { eth0: [ipv6] }, expected: 'http://localhost:8789' },
  { name: 'IPv6 wildcard retains IPv6 advertisement', host: '::',
    interfaces: { eth0: [ipv6] }, expected: 'http://[2001:db8::7]:8789' },
  { name: 'bracketed IPv6 wildcard retains IPv6 advertisement', host: '[::]',
    interfaces: { eth0: [ipv6] }, expected: 'http://[2001:db8::7]:8789' },
  { name: 'IPv4 wildcard prefers a usable IPv4 interface', host: '0.0.0.0',
    interfaces: { eth0: [ipv6, ipv4] }, expected: 'http://192.168.1.7:8789' },
  { name: 'private-only mode retains its existing IPv4 fallback', host: '0.0.0.0',
    interfaces: { eth0: [ipv6] }, privateNetworkOnly: true, expected: 'http://localhost:8789' },
  { name: 'an explicit IPv6 bind is not replaced by an interface address', host: '::1',
    interfaces: { eth0: [ipv4] }, expected: 'http://[::1]:8789' },
];

for (const fixture of cases) {
  test(fixture.name, { timeout: 1000 }, context => {
    const original = JSON.stringify(fixture.interfaces);
    const options = { interfaces: fixture.interfaces,
      privateNetworkOnly: fixture.privateNetworkOnly ?? false };
    const origin = resolveHeadlessConnectionString(fixture.host, 8789, options);
    const pairing = buildPairingUrl(origin);
    context.diagnostic(JSON.stringify({ node: process.version, sourceDigest,
      host: fixture.host, origin, pairing, syntheticInterfaces: true }));
    assert.equal(JSON.stringify(fixture.interfaces), original);
    assert.equal(origin, fixture.expected);
    assert.equal(new URL(pairing).origin, fixture.expected);
    assert.equal(new URL(pairing).pathname, '/pair');
    assert.equal(new URL(pairing).search, '');
    assert.equal(new URL(pairing).hash, '');
  });
}
