import assert from 'node:assert/strict';
import { join } from 'node:path';
import test from 'node:test';
import { pathToFileURL } from 'node:url';

const url = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ? pathToFileURL(join(process.env.FRAMERIZE_SERVER_PACKAGE_ROOT, 'dist/tailscale.js'))
  : new URL('../dist/tailscale.js', import.meta.url);
const { prepareTailscale } = await import(url.href);

for (const action of ['apply', 'remove']) test(`cancelled Tailscale ${action} still permits rollback`, async () => {
  const controller = new AbortController();
  const reason = new Error('synthetic cancellation');
  const target = 'http://127.0.0.1:8789';
  const prior = action === 'remove' ? target : undefined;
  let route = prior, cancelNext = true;
  const run = async (_command, args, signal) => {
    assert.ok(!signal?.aborted, 'rollback must not inherit the aborted signal');
    if (args[0] === 'status') return { code: 0, stdout: JSON.stringify({ Self: {
      DNSName: 'synthetic.example.ts.net.', CapMap: { https: [] },
    } }) };
    if (args[1] === 'status') return { code: 0, stdout: JSON.stringify(route ? {
      Web: { 'synthetic.example.ts.net:443': { Handlers: { '/': { Proxy: route } } } },
    } : {}) };
    if (args.includes('--bg')) route = args.at(-1);
    if (args.at(-1) === 'off') route = undefined;
    if (cancelNext) { cancelNext = false; controller.abort(reason); }
    return { code: 0, stdout: '' };
  };
  const prepared = await prepareTailscale(8789, 443, run, '127.0.0.1', undefined, controller.signal);
  await assert.rejects(prepared[action](), error => error === reason);
  assert.equal(route, action === 'apply' ? target : undefined);
  await prepared.rollback();
  assert.equal(route, prior);
});

test('an already cancelled Tailscale preparation starts no command', async () => {
  const reason = new Error('already cancelled');
  const run = async () => assert.fail('No command should start');
  await assert.rejects(prepareTailscale(8789, 443, run, '127.0.0.1', undefined,
    AbortSignal.abort(reason)), error => error === reason);
});
