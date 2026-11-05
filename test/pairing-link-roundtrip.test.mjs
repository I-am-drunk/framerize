import test from 'node:test';
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { Agent, createServer, get } from 'node:http';
import { pathToFileURL } from 'node:url';

const serverRoot = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ? pathToFileURL(`${process.env.FRAMERIZE_SERVER_PACKAGE_ROOT}/`)
  : new URL('../framerize-server/', import.meta.url);
const remoteRoot = process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ? pathToFileURL(`${process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT}/`)
  : new URL('../framerize-remote/', import.meta.url);

function requestStatus(url, agent = false) {
  return new Promise((resolve, reject) => {
    const outgoing = get(url, { agent, signal: AbortSignal.timeout(2000) }, response => {
      response.once('error', reject);
      response.once('end', () => resolve(response.statusCode));
      response.resume();
    });
    outgoing.once('error', reject);
  });
}

test('pairing links round-trip between Server and Remote without sending the fragment',
  { timeout: 8000 }, async context => {
    const { buildPairingUrl } = await import(new URL('dist/startup-access.js', serverRoot));
    const { parsePairingLink } = await import(new URL('dist/index.js', remoteRoot));
    assert.equal(typeof buildPairingUrl, 'function');
    assert.equal(typeof parsePairingLink, 'function');
    const origins = [
      ['http://127.0.0.1:43123', 'http://127.0.0.1:43123'],
      ['http://[::1]:43123/', 'http://[::1]:43123'],
      ['HTTPS://SYNTHETIC.EXAMPLE:443/', 'https://synthetic.example'],
      ['https://synthetic.example:8443', 'https://synthetic.example:8443'],
    ];
    const combinedToken = 'synthetic +%2B&=#/ spaces \u96ea\u{1f9ea}';
    const tokens = [undefined, null, '', 'synthetic-plain', 'synthetic+plus',
      'synthetic%2Bpercent', 'synthetic&and', 'synthetic=equals==', 'synthetic#hash',
      'synthetic/slash', 'synthetic two words', ' ', 'synthetic\u96ea\u{1f9ea}', combinedToken];
    function roundTrip(connectionString, canonical, token) {
      const link = token === undefined ? buildPairingUrl(connectionString)
        : buildPairingUrl(connectionString, token);
      const expectedToken = token === null || token === '' ? undefined : token;
      const fragment = expectedToken === undefined ? ''
        : `#${new URLSearchParams({ token: expectedToken })}`;
      assert.equal(link, `${canonical}/pair${fragment}`);
      const url = new URL(link);
      assert.equal(url.origin, canonical);
      assert.equal(url.pathname, '/pair');
      assert.equal(url.search, '');
      assert.equal(url.hash, fragment);
      const parsed = parsePairingLink(link);
      assert.equal(parsed.remote, canonical);
      assert.equal(parsed.token, expectedToken);
      return link;
    }
    for (const [connectionString, canonical] of origins) for (const token of tokens) {
      roundTrip(connectionString, canonical, token);
    }
    const requests = [];
    const server = createServer((request, response) => {
      requests.push(request.url);
      request.resume();
      response.writeHead(200, { 'content-type': 'text/plain' });
      response.end('synthetic pairing page');
    });
    const agent = new Agent({ keepAlive: false, proxyEnv: {} });
    async function close() {
      agent.destroy();
      if (!server.listening) return;
      const closed = once(server, 'close', { signal: AbortSignal.timeout(2000) });
      server.close();
      server.closeAllConnections();
      await closed;
    }
    context.after(close);
    const listening = once(server, 'listening', { signal: AbortSignal.timeout(2000) });
    server.listen(0, '127.0.0.1');
    await listening;
    const origin = `http://127.0.0.1:${server.address().port}`;
    const pairingUrl = roundTrip(origin, origin, combinedToken);
    assert.equal(await requestStatus(pairingUrl, agent), 200);
    assert.deepEqual(requests, ['/pair']);
    await close();
    assert.equal(server.listening, false);
    await assert.rejects(requestStatus(`${origin}/pair`), { code: 'ECONNREFUSED' });
  });
