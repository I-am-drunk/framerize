import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { runClient } from './command.ts'; export { nodeRunner } from './process.ts';",
  resolveDir: fileURLToPath(new URL('../src/compatibility/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
const { runClient, nodeRunner } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
console.log(JSON.stringify({ runtime: process.version, bundle: createHash('sha256')
  .update(compiled.outputFiles[0].contents).digest('hex') }));

const excluded = ['ANTHROPIC_API_KEY', 'CLAUDE_CODE_USE_BEDROCK', 'CLAUDE_CODE_USE_VERTEX',
  'CLAUDE_CODE_USE_FOUNDRY', 'CLAUDE_CODE_USE_ANTHROPIC_AWS'];
const origin = 'http://127.0.0.1:8788';
const token = 'synthetic-local-token';

async function launch(platform, inherited, client = 'claude', authenticated = true) {
  const calls = [], output = [], captures = [], reads = [];
  const env = { PATH: platform === 'win32' ? 'C:\\synthetic\\bin' : '/synthetic/bin',
    PATHEXT: '.EXE', AWS_REGION: 'synthetic-region', KEEP_ME: 'unchanged', ...inherited };
  const before = { ...env };
  const signals = ['SIGINT', 'SIGTERM'].map(name => process.listenerCount(name));
  const runner = nodeRunner(platform, (command, args, options) => {
    calls.push({ command, args, options });
    const child = new EventEmitter();
    child.kill = () => assert.fail('No signal should be sent to the synthetic child');
    queueMicrotask(() => child.emit('close', 0, null));
    return child;
  });
  runner.capture = async command => { captures.push(command); return null; };
  const code = await runClient([client, '--', '--model', 'synthetic-model'], {
    platform, env, home: '/synthetic/home', stateDir: '/synthetic/state', cwd: '/synthetic/project',
    target: { origin, ...(authenticated ? { localToken: token } : {}) }, runner,
    probe: candidate => candidate === (platform === 'win32'
      ? `C:\\synthetic\\bin\\${client}.EXE` : `/synthetic/bin/${client}`),
    realPath: path => path, read: async path => { reads.push(path); return null; },
    fetch: async () => assert.fail('No release fetch is allowed'),
    io: { stdout: text => output.push(text), stderr: text => output.push(text) },
  });
  assert.equal(code, 0);
  assert.equal(calls.length, 1);
  assert.deepEqual(output, []);
  assert.deepEqual(env, before, 'The parent environment must remain unchanged');
  assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => process.listenerCount(name)), signals);
  assert.equal(captures.length, platform === 'win32' || client === 'codex' ? 0 : 1);
  assert.equal(reads.length > 0, client === 'claude');
  assert.equal(calls[0].options.env.AWS_REGION, 'synthetic-region');
  assert.equal(calls[0].options.env.KEEP_ME, 'unchanged');
  if (client === 'claude') {
    assert.deepEqual(calls[0].args, ['--model', 'synthetic-model']);
    assert.equal(calls[0].options.env.ANTHROPIC_BASE_URL, `${origin}/anthropic`);
    assert.equal(calls[0].options.env.ANTHROPIC_AUTH_TOKEN,
      authenticated ? token : 'framerize-no-local-token');
  }
  return calls[0].options.env;
}

const mixed = Object.fromEntries(excluded.map(key => [key[0] + key.slice(1).toLowerCase(), '1']));
const upper = Object.fromEntries(excluded.map(key => [key, '1']));
const lower = Object.fromEntries(excluded.map(key => [key.toLowerCase(), '1']));

for (const [name, inherited, authenticated] of [
  ['mixed-case cloud keys', mixed, true],
  ['duplicate-case cloud keys without a local token', { ...upper, ...lower }, false],
  ['uppercase cloud keys', upper, true],
]) {
  test(`Windows launch environment removes ${name}`, { timeout: 3000 }, async () => {
    const env = await launch('win32', inherited, 'claude', authenticated);
    assert.deepEqual(Object.keys(env).filter(key => excluded.includes(key.toUpperCase())), []);
  });
}

for (const platform of ['linux', 'darwin']) {
  test(`${platform} preserves distinct lowercase names`, { timeout: 3000 }, async () => {
    const env = await launch(platform, { ...upper, ...lower });
    assert.deepEqual(Object.keys(env).filter(key => excluded.includes(key)), []);
    for (const key of Object.keys(lower)) assert.equal(env[key], '1');
  });
}

test('Codex retains unrelated inherited provider keys', { timeout: 3000 }, async () => {
  const inherited = { ...upper, ...lower, ...mixed };
  const env = await launch('win32', inherited, 'codex');
  for (const key of Object.keys(inherited)) assert.equal(env[key], inherited[key]);
  assert.equal(env.FRM_LOCAL_TOKEN, token);
});
