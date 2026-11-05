import assert from 'node:assert/strict';
import test from 'node:test';
import { prepareTailscale } from '../dist/tailscale.js';

function fixture(initial) {
  const state = { route: initial, calls: [], https: true };
  const run = async (_command, args) => {
    state.calls.push([...args]);
    if (args[0] === 'status') return { code: 0, stdout: JSON.stringify({ Self: {
      DNSName: 'fixture.example.ts.net.', TailscaleIPs: ['100.90.1.2'], CapMap: state.https ? { https: [] } : {},
    } }) };
    if (args[1] === 'status') return { code: 0, stdout: JSON.stringify(state.route ? {
      TCP: { 443: { HTTPS: true } }, Web: { 'fixture.example.ts.net:443': {
        Handlers: { '/': { Proxy: state.route } },
      } },
    } : {}) };
    if (args.includes('--bg')) state.route = args.at(-1);
    if (args.at(-1) === 'off') state.route = undefined;
    return { code: 0, stdout: '' };
  };
  return { state, run };
}

test('Tailscale creates and rolls back only the route this setup added', async () => {
  const { state, run } = fixture();
  const route = await prepareTailscale(8789, 443, run);
  assert.equal(route.origin, 'https://fixture.example.ts.net');
  assert.equal(state.route, undefined);
  await route.apply();
  assert.equal(state.route, 'http://127.0.0.1:8789');
  await route.rollback();
  assert.equal(state.route, undefined);
});

test('Tailscale refuses an occupied route before mutation', async () => {
  const { state, run } = fixture('http://127.0.0.1:9999');
  await assert.rejects(prepareTailscale(8789, 443, run), /another application/);
  assert.equal(state.route, 'http://127.0.0.1:9999');
  assert.ok(state.calls.every(args => !args.includes('--bg') && !args.includes('off')));
});

test('an existing matching route survives rollback, and explicit removal can be rolled back', async () => {
  const { state, run } = fixture('http://127.0.0.1:8789');
  const route = await prepareTailscale(8789, 443, run);
  await route.apply();
  await route.rollback();
  assert.equal(state.route, 'http://127.0.0.1:8789');
  assert.ok(state.calls.every(args => !args.includes('--bg') && !args.includes('off')));
  await route.remove();
  assert.equal(state.route, undefined);
  await route.rollback();
  assert.equal(state.route, 'http://127.0.0.1:8789');
});

test('disabled HTTPS prevents Tailscale mutation', async () => {
  const { state, run } = fixture();
  state.https = false;
  await assert.rejects(prepareTailscale(8789, 443, run), /Enable HTTPS/);
  assert.ok(state.calls.every(args => !args.includes('--bg')));
});

test('a new route owner between preparation and apply is preserved', async () => {
  const { state, run } = fixture();
  const route = await prepareTailscale(8789, 443, run);
  state.route = 'http://127.0.0.1:9999';
  await assert.rejects(route.apply(), /another application/);
  await route.rollback();
  assert.equal(state.route, 'http://127.0.0.1:9999');
  assert.ok(state.calls.every(args => !args.includes('--bg') && !args.includes('off')));
});

test('an already configured route at apply is not owned by this attempt', async () => {
  const { state, run } = fixture();
  const route = await prepareTailscale(8789, 443, run);
  state.route = 'http://127.0.0.1:8789';
  await route.apply();
  await route.rollback();
  assert.equal(state.route, 'http://127.0.0.1:8789');
  assert.ok(state.calls.every(args => !args.includes('--bg') && !args.includes('off')));
});

test('rollback preserves a route replaced by another application', async () => {
  const { state, run } = fixture();
  const route = await prepareTailscale(8789, 443, run);
  await route.apply();
  state.route = 'http://127.0.0.1:9999';
  await route.rollback();
  assert.equal(state.route, 'http://127.0.0.1:9999');
  assert.ok(state.calls.every(args => !args.includes('off')));
});

test('restoring a removed route refuses a new owner', async () => {
  const { state, run } = fixture('http://127.0.0.1:8789');
  const route = await prepareTailscale(8789, 443, run);
  await route.remove();
  state.route = 'http://127.0.0.1:9999';
  await assert.rejects(route.rollback(), /Another application/);
  assert.equal(state.route, 'http://127.0.0.1:9999');
});

test('a saved route can move to a new local port and roll back', async () => {
  const oldTarget = 'http://127.0.0.1:8789';
  const { state, run } = fixture(oldTarget);
  const route = await prepareTailscale(8790, 443, run, '127.0.0.1', oldTarget);
  await route.apply();
  assert.equal(state.route, 'http://127.0.0.1:8790');
  await route.rollback();
  assert.equal(state.route, oldTarget);
});

test('saved route ownership does not authorize a replacement by another owner', async () => {
  const oldTarget = 'http://127.0.0.1:8789';
  const { state, run } = fixture(oldTarget);
  const route = await prepareTailscale(8790, 443, run, '127.0.0.1', oldTarget);
  state.route = 'http://127.0.0.1:9999';
  await assert.rejects(route.apply(), /another application/);
  assert.equal(state.route, 'http://127.0.0.1:9999');
});
