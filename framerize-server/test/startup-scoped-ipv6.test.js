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
const bundleSha256 = createHash('sha256').update(bytes).digest('hex');
const { resolveHeadlessConnectionString, buildPairingUrl } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
const ipv6 = (address, scopeid = 0) => ({ address, scopeid, internal: false,
  family: 'IPv6', netmask: 'ffff:ffff:ffff:ffff::', mac: '00:00:00:00:00:00', cidr: `${address}/64` });
const scoped = ipv6('fe80::7', 4);
const global = ipv6('2001:db8::7');
const privateAddress = ipv6('fd12:3456:789a::7');
const ipv4 = { address: '192.168.1.7', internal: false, family: 'IPv4',
  netmask: '255.255.255.0', mac: '00:00:00:00:00:00', cidr: '192.168.1.7/24' };
const cases = ['::', '[::]'].flatMap(host => [
  { name: `${host} skips an interface-scoped address before a global IPv6 address`, host,
    interfaces: { eth0: [scoped, global] }, expected: 'http://[2001:db8::7]:8789' },
  { name: `${host} falls back when only an interface-scoped address exists`, host,
    interfaces: { eth0: [scoped] }, expected: 'http://localhost:8789' },
]);
cases.push(
  { name: 'an unscoped IPv6 candidate remains selected before a scoped address', host: '::',
    interfaces: { eth0: [global, scoped] }, expected: 'http://[2001:db8::7]:8789' },
  { name: 'IPv4 remains preferred with a scoped IPv6 address first', host: '::',
    interfaces: { eth0: [scoped, ipv4, global] }, expected: 'http://192.168.1.7:8789' },
  { name: 'private-only selection retains the unscoped ULA', host: '::', privateNetworkOnly: true,
    interfaces: { eth0: [scoped, global, privateAddress] }, expected: 'http://[fd12:3456:789a::7]:8789' },
  { name: 'an explicit IPv6 bind ignores the interface candidates', host: '::1',
    interfaces: { eth0: [scoped, global] }, expected: 'http://[::1]:8789' },
);

for (const fixture of cases) {
  test(fixture.name, { timeout: 1000 }, context => {
    const before = structuredClone(fixture.interfaces);
    const origin = resolveHeadlessConnectionString(fixture.host, 8789, {
      interfaces: fixture.interfaces, privateNetworkOnly: fixture.privateNetworkOnly ?? false,
    });
    const pairing = new URL(buildPairingUrl(origin));
    context.diagnostic(JSON.stringify({ node: process.version, bundleSha256,
      host: fixture.host, origin, pairing: pairing.href, syntheticInterfaces: true }));
    assert.deepEqual(fixture.interfaces, before);
    assert.equal(pairing.origin, origin);
    assert.equal(pairing.pathname, '/pair');
    assert.equal(pairing.search, '');
    assert.equal(pairing.hash, '');
    assert.equal(origin, fixture.expected);
  });
}
