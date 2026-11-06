import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { getEventListeners } from 'node:events';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/tailscale.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
});
const bytes = compiled.outputFiles[0].contents;
const { isTailscaleIpv4Address, parseTailscaleStatus, readTailscaleStatus } =
  await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
console.log(JSON.stringify({ runtime: process.version, platform: process.platform,
  inputs: Object.keys(compiled.metafile.inputs).sort(),
  bundleSha256: createHash('sha256').update(bytes).digest('hex') }));

const cases = [
  ['100.64junk.0.1', false],
  ['100.64.0.1suffix', false],
  ['100.64.0.1/32', false],
  ['100.64.0.1', true],
  ['100.127.255.254', true],
  ['100.128.0.1', false],
  ['192.168.1.1', false],
  ['fd7a:115c:a1e0::1', false],
];

for (const [address, expected] of cases) {
  test(`Tailscale status validates the entire IPv4 address: ${address}`, { timeout: 6000 }, async context => {
    const status = JSON.stringify({ BackendState: 'Running', Self: { TailscaleIPs: [address] } });
    const controller = new AbortController();
    let child;
    let closed;
    let launches = 0;
    context.after(async () => {
      controller.abort();
      if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
      if (closed) await bounded(closed);
    });
    const launch = (command, args, options) => {
      launches += 1;
      assert.equal(command, process.platform === 'win32' ? 'tailscale.exe' : 'tailscale');
      assert.deepEqual(args, ['status', '--json']);
      assert.equal(options.shell, false);
      child = spawn(process.execPath, ['-e', `process.stdout.write(${JSON.stringify(status)})`],
        { env: {}, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
      closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
      return child;
    };
    const parsed = parseTailscaleStatus(status);
    const result = await readTailscaleStatus({ env: {}, signal: controller.signal, launch });
    assert.equal(launches, 1);
    assert.equal(typeof child.pid, 'number');
    assert.deepEqual(await bounded(closed), { code: 0, signal: null });
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    context.diagnostic(JSON.stringify({ address, expected, helper: isTailscaleIpv4Address(address),
      parsed: parsed.tailnetIpv4Addresses, state: result.state, childClosed: true }));
    assert.equal(result.scope, 'local_only');
    assert.equal(result.state, expected ? 'running' : 'no_address');
    assert.equal(isTailscaleIpv4Address(address), expected);
    assert.deepEqual(parsed.tailnetIpv4Addresses, expected ? [address] : []);
  });
}

async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('synthetic child did not close')), 2500);
    })]);
  } finally { clearTimeout(timer); }
}
