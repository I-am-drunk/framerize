import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { getEventListeners } from 'node:events';
import { fileURLToPath } from 'node:url';
import { setImmediate } from 'node:timers/promises';
import { build } from 'esbuild';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/tailscale.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const { readTailscaleStatus } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const canary = 'synthetic-private-canary';
const env = { PATH: '/synthetic/bin', NODE_OPTIONS: canary, SECRET: canary, HTTP_PROXY: canary };
test('an asynchronous spawn failure releases its timer and listener', async context => {
  const root = await mkdtemp(join(tmpdir(), 'remote-tailscale-spawn-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const controller = new AbortController();
  let closed = false;
  const result = await readTailscaleStatus({ signal: controller.signal, env,
    launch(command, args, options) {
      const child = spawn(join(root, 'absent-executable'), args, options);
      child.once('close', () => { closed = true; }); return child;
    },
  });
  assert.equal(result.state, 'missing'); assert.equal(closed, true);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});
for (const mode of ['timeout', 'cancelled', 'output_limit']) {
  test(`Tailscale ${mode} reaps the child and removes listeners`, { timeout: 6000 }, async () => {
    const controller = new AbortController();
    const before = process.getActiveResourcesInfo().filter(name => /Process|Timeout/.test(name)).sort();
    let closed = false;
    const script = mode === 'output_limit'
      ? "process.stdout.write('x'.repeat(140000), () => process.stderr.write('x'.repeat(140000))); setInterval(() => {}, 100);"
      : "process.stdout.write('started'); setInterval(() => {}, 100);";
    const result = await readTailscaleStatus({ env, signal: controller.signal,
      launch: synthetic(script, child => {
        child.once('close', () => { closed = true; });
        if (mode === 'cancelled') child.stdout.once('data', () => controller.abort());
      }),
    });
    assert.equal(result.state, mode); assert.equal(closed, true);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    await setImmediate(); await setImmediate();
    assert.deepEqual(process.getActiveResourcesInfo().filter(name => /Process|Timeout/.test(name)).sort(), before);
  });
}
const statuses = [
  [{ BackendState: 'Running', Self: { TailscaleIPs: ['100.64.0.1'], DNSName: canary } }, 'running'],
  [{ BackendState: 'Running', Self: { TailscaleIPs: ['100.127.255.255'] } }, 'running'],
  [{ BackendState: 'Running', Self: { TailscaleIPs: ['100.128.0.1', '100.64.0.1.bad', '100.64.0.999'] } }, 'no_address'],
  [{ BackendState: 'NeedsLogin', AuthURL: canary }, 'needs_login'],
  [{ BackendState: 'NeedsMachineAuth' }, 'needs_login'], [{ BackendState: 'Stopped' }, 'not_running'],
  [{ BackendState: 'Unknown', Self: { TailscaleIPs: ['100.64.0.1'] } }, 'invalid_response'],
  [null, 'invalid_response'], ['invalid json', 'invalid_response'],
];
for (const [status, expected] of statuses) {
  test(`Tailscale reports ${expected} from synthetic status`, async () => {
    const raw = typeof status === 'string' ? status : JSON.stringify(status);
    const result = await readTailscaleStatus({ env,
      launch: synthetic(`process.stdout.write(${JSON.stringify(raw)})`) });
    assert.equal(result.state, expected); assert.equal(result.scope, 'local_only');
    assert.doesNotMatch(JSON.stringify(result), /synthetic-private|100\.64|AuthURL|DNSName/);
  });
}

function synthetic(script, observe = () => {}, platform = process.platform) {
  return (command, args, options) => {
    assert.equal(command, platform === 'win32' ? 'tailscale.exe' : 'tailscale');
    assert.deepEqual(args, ['status', '--json']);
    assert.equal(options.shell, false); assert.equal(options.windowsHide, true);
    assert.deepEqual(options.env, { PATH: env.PATH });
    const child = spawn(process.execPath, ['-e', script], options);
    observe(child); return child;
  };
}

test('pre-aborted Tailscale check does not spawn', async () => {
  const controller = new AbortController(); controller.abort();
  const result = await readTailscaleStatus({ signal: controller.signal,
    launch() { assert.fail('must not spawn'); } });
  assert.equal(result.state, 'cancelled');
});

for (const [code, expected] of [['ENOENT', 'missing'], ['EACCES', 'permission_denied'], ['UNKNOWN', 'failed']]) {
  test(`Tailscale safely classifies synchronous ${code}`, async () => {
    const result = await readTailscaleStatus({ launch() { throw Object.assign(new Error(canary), { code }); } });
    assert.equal(result.state, expected); assert.doesNotMatch(JSON.stringify(result), /synthetic-private/);
  });
}

for (const [stderr, expected] of [['not logged in', 'needs_login'],
  ['Permission denied', 'permission_denied'], ['unknown error', 'failed']]) {
  test(`Tailscale maps stderr to ${expected} without exposing it`, async () => {
    const script = `process.stderr.write(${JSON.stringify(`${stderr} ${canary}`)}); process.exitCode = 1;`;
    const result = await readTailscaleStatus({ env, launch: synthetic(script) });
    assert.equal(result.state, expected);
    assert.doesNotMatch(JSON.stringify(result), /synthetic-private/);
  });
}

test('Windows command selection stays direct and strips ambient environment', async () => {
  const result = await readTailscaleStatus({ env, platform: 'win32',
    launch: synthetic('process.stdout.write("null")', undefined, 'win32') });
  assert.equal(result.state, 'invalid_response');
});
