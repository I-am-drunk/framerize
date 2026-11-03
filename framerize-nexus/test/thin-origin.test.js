import test from 'node:test';
import assert from 'node:assert/strict';
import { INVALID_UPSTREAM, normalizeUpstream } from '../src/thin/origin.js';

test('normalizeUpstream returns the bare origin for origin and /v1 forms', () => {
  const cases = [
    ['https://api.example.test', 'https://api.example.test'],
    ['https://api.example.test/', 'https://api.example.test'],
    ['https://api.example.test/v1', 'https://api.example.test'],
    ['https://api.example.test/v1/', 'https://api.example.test'],
    ['http://127.0.0.1:8787', 'http://127.0.0.1:8787'],
    ['http://127.0.0.1:8787/v1/', 'http://127.0.0.1:8787'],
    ['http://[::1]:9000/v1', 'http://[::1]:9000'],
    ['https://[2001:db8::1]/', 'https://[2001:db8::1]'],
    ['HTTPS://API.Example.TEST:443/v1', 'https://api.example.test'],
    ['http://localhost:80/', 'http://localhost'],
    ['https://a.test./v1', 'https://a.test.'],
    ['https://bücher.example/v1', 'https://xn--bcher-kva.example'],
    ['https://a.test:0', 'https://a.test:0'],
    ['https://a.test:0443/v1', 'https://a.test'],
    ['http://a.test:080', 'http://a.test'],
    ['http://a.test:', 'http://a.test'],
    ['http://2130706433', 'http://127.0.0.1'],
    ['http://0x7f.1', 'http://127.0.0.1'],
    ['http://127.1', 'http://127.0.0.1'],
    ['http://127.000.000.001/v1', 'http://127.0.0.1'],
    ['http://0x7f000001/v1', 'http://127.0.0.1'],
    ['https://a_b.test', 'https://a_b.test'],
    ['https://.a.test', 'https://.a.test'],
    ['https://a..test', 'https://a..test'],
    ['https://-a.test', 'https://-a.test'],
    ['https://a-.test', 'https://a-.test'],
    ['http://[2001:db8:0:0::1]/v1', 'http://[2001:db8::1]'],
  ];
  for (const [input, expected] of cases) {
    assert.equal(normalizeUpstream(input), expected);
  }
});

function rejects(input) {
  assert.throws(() => normalizeUpstream(input), (error) => {
    assert.equal(error.message, INVALID_UPSTREAM);
    return true;
  });
}

test('normalizeUpstream rejects nonstring and empty input', () => {
  for (const input of [undefined, null, 0, 1, true, {}, [], ['https://a.test']]) {
    rejects(input);
  }
  for (const input of ['', ' ', 'https://', 'https:///v1', 'a.test', '//a.test',
    'https:example.test/v1', 'https:/example.test/v1']) {
    rejects(input);
  }
});

test('normalizeUpstream rejects other protocols and paths', () => {
  const inputs = [
    'ftp://a.test', 'ws://a.test', 'file:///v1', 'httpx://a.test',
    'https://a.test/v1/v1', 'https://a.test/v2', 'https://a.test/v1/chat',
    'https://a.test/v1/chat/completions', 'https://a.test/V1', 'https://a.test//',
    'https://a.test//v1', 'https://a.test/./v1', 'https://a.test/x/../v1',
    'https://a.test/%761', 'https://a.test\\v1', 'https://a.test/v1//',
    'https://a.test/%2e/v1', 'https://a.test/\nv1',
  ];
  for (const input of inputs) rejects(input);
});

test('normalizeUpstream rejects userinfo, query and fragment including empty delimiters', () => {
  const inputs = [
    'https://user:pass@a.test', 'https://user@a.test', 'https://@a.test',
    'https://:@a.test/v1', 'https://a.test?', 'https://a.test/v1?',
    'https://a.test/?x=1', 'https://a.test#', 'https://a.test/v1#',
    'https://a.test/#frag', 'https://a.test/v1?#',
  ];
  for (const input of inputs) rejects(input);
});

test('normalizeUpstream rejects malformed URLs, out-of-range ports and raw controls', () => {
  const inputs = [
    ' https://a.test', 'https://a.test ', 'https://a.test\n', 'https://a.\ttest',
    'https://a.test:invalid', 'https://a.test:-1', 'https://a.test:1.5',
    'https://a.test\u0000', 'https://a.test\u007f',
    'https://a.test:65536', 'https://a.test:99999', 'http://[::1', 'http://[zz::1]',
    'http://[::1]x', 'http://::1', 'http://[fe80::1%25eth0]',
  ];
  for (const input of inputs) rejects(input);
});

test('normalizeUpstream errors never echo synthetic secrets or inputs', () => {
  const secret = 'SYNTHETIC_SECRET_7f3a';
  const inputs = [
    `https://user:${secret}@a.test/v1`, `https://a.test/v1?key=${secret}`,
    `https://a.test/#${secret}`, `https://a.test/${secret}`, `${secret}://a.test`,
    `https://${secret}.test/v1/v1`, `/home/${secret}/config.json`, secret,
    `https://${secret}.test:65536/v1`,
  ];
  for (const input of inputs) {
    let caught;
    try { normalizeUpstream(input); } catch (error) { caught = error; }
    assert.ok(caught instanceof Error);
    assert.equal(caught.message, INVALID_UPSTREAM);
    assert.equal(caught.cause, undefined);
    const text = `${String(caught)} ${caught.stack} ${JSON.stringify(caught)}`;
    assert.equal(text.includes(secret), false);
    assert.equal(text.includes(input), false);
  }
});
