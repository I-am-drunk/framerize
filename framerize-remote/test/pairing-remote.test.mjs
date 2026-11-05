// Tailscale cases adapted from T3 Code packages/tailscale/src/tailscale.test.ts and host cases from
// packages/shared/src/hostClassification.test.ts at 5a906583 (MIT, T3 Tools Inc.).
import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const source = name => JSON.stringify(fileURLToPath(new URL(`../src/${name}.ts`, import.meta.url)));
const compiled = await build({ stdin: { contents: ['pairingLink', 'tailscale', 'hostClassification']
  .map(name => `export * from ${source(name)};`).join('\n'), resolveDir: fileURLToPath(new URL('.', import.meta.url)) },
bundle: true, platform: 'node', format: 'esm', write: false });
const remote = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

const tailscaleStatusJson = '{"Self":{"DNSName":"desktop.tail.ts.net.","TailscaleIPs":["100.100.100.100","fd7a:115c:a1e0::1","192.168.1.20"]}}';

test('parses status facts', () => {
  assert.deepEqual(remote.parseTailscaleStatus(tailscaleStatusJson), {
    magicDnsName: 'desktop.tail.ts.net', tailnetIpv4Addresses: ['100.100.100.100'], httpsServeEnabled: null });
});

test('reads the HTTPS serve capability from the status capability map', () => {
  assert.equal(remote.parseTailscaleStatus('{"Self":{"CapMap":{"https":null,"funnel":null}}}').httpsServeEnabled, true);
  assert.equal(remote.parseTailscaleStatus('{"Self":{"CapMap":{"funnel":null}}}').httpsServeEnabled, false);
  assert.throws(() => remote.parseTailscaleStatus('[]'));
});

test('builds the Tailscale HTTPS base URL', () => {
  assert.equal(remote.buildTailscaleHttpsBaseUrl({ magicDnsName: 'desktop.tail.ts.net' }), 'https://desktop.tail.ts.net/');
  assert.equal(remote.buildTailscaleHttpsBaseUrl({ magicDnsName: 'desktop.tail.ts.net', servePort: 8443 }),
    'https://desktop.tail.ts.net:8443/');
});

test('private-network hosts use T3 classification, including the Tailscale range edges', () => {
  for (const host of ['10.0.0.1', '127.0.0.1', '192.168.1.10', '172.31.255.255', '169.254.1.1',
    '100.64.0.1', '100.127.255.255', 'air', 'printer.local', 'box.tailnet.ts.net', 'AIR.TAILE8BEA7.TS.NET',
    '::1', '[::1]', 'fd00::1', 'fe80::1']) assert.equal(remote.isPrivateNetworkHost(host), true, host);
  for (const host of ['100.63.255.255', '100.128.0.1', '8.8.8.8', 'github.com', '2606:4700:4700::1111']) {
    assert.equal(remote.isPrivateNetworkHost(host), false, host);
  }
  assert.equal(remote.isTailnetHost('desktop.tail.ts.net'), true);
  assert.equal(remote.isTailnetHost('100.100.100.100'), false);
});

test('parsePairingLink returns the origin and the exact decoded token once', () => {
  const link = (origin, token) => `${origin}/pair#${new URLSearchParams([['token', token]])}`;
  for (const token of ['plain.token-1', 'a+b/c=', 'p%c&d=e#f/g', 'ünï-cødé']) {
    assert.deepEqual(remote.parsePairingLink(link('http://100.64.0.10:8787', token)),
      { remote: 'http://100.64.0.10:8787', token });
  }
  assert.deepEqual(remote.parsePairingLink('https://desktop.tail.ts.net/pair'), { remote: 'https://desktop.tail.ts.net' });
  assert.deepEqual(remote.parsePairingLink('https://desktop.tail.ts.net/pair#token='), { remote: 'https://desktop.tail.ts.net' });
  assert.equal('token' in remote.parsePairingLink('http://10.0.0.2/pair'), false);
});

test('parsePairingLink rejects other schemes, userinfo, paths, queries and repeated tokens', () => {
  for (const input of ['ftp://server/pair#token=a', 'http://user:pw@server/pair#token=a', 'http://server/#token=a',
    'http://server/pair/extra#token=a', 'http://server/pair?token=a', 'http://server/pair?#token=a',
    'http://server/pair#token=a&token=b', 'http://server/pair#token=a&other=b', 'not a url']) {
    assert.throws(() => remote.parsePairingLink(input), { code: 'pairing_link_invalid' }, input);
  }
});
