// Adapted from T3 Code startupAccess.test.ts at 5a906583. MIT, T3 Tools Inc.
import assert from 'node:assert/strict';
import test from 'node:test';
import { acceptsPrivateNetworkConnection } from '../dist/private-network.js';
import { resolveHeadlessConnectionString } from '../dist/startup-access.js';

const ipv4 = (address, internal = false) => ({ address, netmask: '255.255.255.0',
  family: 'IPv4', mac: '00:00:00:00:00:00', internal, cidr: `${address}/24` });

test('LAN advertisement skips public and virtual interfaces and prefers LAN over tailnet', () => {
  const interfaces = { eth0: [ipv4('203.0.113.7')], docker0: [ipv4('172.17.0.1')],
    bridge200: [ipv4('192.168.64.1')], tailscale0: [ipv4('100.90.1.2')],
    eth1: [ipv4('10.0.0.5')], lo: [ipv4('127.0.0.1', true)] };
  assert.equal(resolveHeadlessConnectionString('0.0.0.0', 8789, { interfaces, privateNetworkOnly: true }),
    'http://10.0.0.5:8789');
  assert.equal(resolveHeadlessConnectionString('0.0.0.0', 8789,
    { interfaces: { docker0: interfaces.docker0 }, privateNetworkOnly: true }), 'http://localhost:8789');
  assert.equal(resolveHeadlessConnectionString('::1', 8789), 'http://[::1]:8789');
});

test('private listener accepts private endpoints and rejects public endpoints on either side', () => {
  const accepts = (localAddress, remoteAddress) => acceptsPrivateNetworkConnection({ localAddress, remoteAddress });
  for (const [local, remote] of [['127.0.0.1', '127.0.0.1'], ['::1', '::1'],
    ['::ffff:192.168.1.20', '::ffff:192.168.1.31'], ['100.101.102.103', '100.90.1.2'],
    ['fd7a:115c:a1e0::1', 'fd7a:115c:a1e0::2']]) assert.equal(accepts(local, remote), true);
  for (const [local, remote] of [['172.31.4.10', '203.0.113.7'], ['203.0.113.7', '192.168.1.31'],
    ['169.254.10.1', '169.254.10.2'], ['::1', '2001:db8::1'], [undefined, '192.168.1.31']]) {
    assert.equal(accepts(local, remote), false);
  }
});
