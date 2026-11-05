import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const source = name => fileURLToPath(new URL(`../src/${name}.ts`, import.meta.url));
const compiled = await build({ stdin: {
  contents: `export { runFrm } from ${JSON.stringify(source('frmCli'))};
export * from ${JSON.stringify(source('diagnostics'))};`,
  resolveDir: fileURLToPath(new URL('.', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false,
plugins: [{ name: 'setup-stage-failures', setup(builder) {
  builder.onResolve({ filter: /^\.\/(cmdUp|clientCli)\.js$/ }, args =>
    args.importer === source('frmCli') ? { path: args.path, namespace: 'fixture' } : undefined);
  builder.onLoad({ filter: /.*/, namespace: 'fixture' }, args => ({ contents:
    args.path.includes('cmdUp') ? `
export async function setUp(input, io, options) {
  io.fail('service', options.signal);
  return { service: 'unchanged' };
}
export function formatUp() { return 'ready\\n'; }` : `
export async function offerClientSetup(path, io, env, deps, signal) {
  io.fail('client', signal);
  return 'skipped';
}
export function runClientCommand() { throw new Error('Unexpected client command'); }` }));
} }] });
const cli = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

for (const stage of ['service', 'client']) for (const event of ['SIGINT', 'SIGTERM']) {
  test(`frm up preserves ${stage} cleanup failure after ${event}`, async t => {
    const root = await mkdtemp(join(tmpdir(), 'frm-cleanup-error-'));
    const previous = process.env.FRM_HOME;
    const counts = ['SIGINT', 'SIGTERM'].map(name => process.listenerCount(name));
    t.after(async () => {
      if (previous === undefined) delete process.env.FRM_HOME;
      else process.env.FRM_HOME = previous;
      await rm(root, { recursive: true, force: true });
    });
    process.env.FRM_HOME = root;
    const cause = new Error('synthetic-private-cleanup-failure');
    const error = stage === 'service' ? new cli.ServiceCommandCleanupError(cause)
      : new cli.ClientCleanupError([cause]);
    let output = '', injected = false;
    const io = { stdout(text) { output += text; }, stderr() {},
      fail(current, signal) {
        if (current !== stage) return;
        injected = true;
        process.emit(event);
        assert.equal(signal.aborted, true);
        throw error;
      } };
    await assert.rejects(cli.runFrm(['up', 'http://127.0.0.1:1'], io,
      { program: ['/synthetic/frm'] }), result => result === error);
    assert.equal(injected, true);
    assert.equal(JSON.parse(cli.commandFailure(error, true)).error, `${stage}_cleanup_failed`);
    assert.doesNotMatch(cli.commandFailure(error, false), /cancelled|synthetic-private/);
    assert.equal(output, stage === 'client' ? 'ready\n' : '');
    assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => process.listenerCount(name)), counts);
  });
}
