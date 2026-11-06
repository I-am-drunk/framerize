import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build } from 'esbuild';

const source = fileURLToPath(new URL('../src/compatibility/conflicts.ts', import.meta.url));
const compiled = await build({ entryPoints: [source], bundle: true, write: false, platform: 'node', format: 'esm' });
const bytes = compiled.outputFiles[0].contents;
const { inspectClaudeSettings, claudeManagedSettingsPath } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
const digest = value => createHash('sha256').update(value).digest('hex');
console.log(JSON.stringify({ runtime: process.version, source: digest(await readFile(source)), bundle: digest(bytes) }));

const userPath = '/synthetic/user/settings.json';
const expected = { ANTHROPIC_BASE_URL: 'http://127.0.0.1:8788/anthropic', ANTHROPIC_AUTH_TOKEN: 'synthetic-local-token' };
const env = { PROGRAMFILES: '/synthetic/programs' };
const managedPath = claudeManagedSettingsPath('win32', env);

async function inspect(values, platform = 'win32', managed = false) {
  const selected = managed ? claudeManagedSettingsPath(platform, env) : userPath;
  return inspectClaudeSettings(async path => path === selected ? JSON.stringify({ env: values }) : null,
    { platform, env, userPath, cwd: '/synthetic/project', expected });
}

test('Windows detects mixed-case token, API key and cloud-provider conflicts', async () => {
  const result = await inspect({
    anthropic_auth_token: 'synthetic-other-token', Anthropic_Api_Key: 'synthetic-api-key',
    claude_code_use_bedrock: '1', Claude_Code_Use_Vertex: '1',
    ClAuDe_CoDe_UsE_FoUnDrY: '1', Claude_Code_Use_Anthropic_Aws: '1',
  });
  assert.deepEqual(result, { overrideBaseUrl: false, blocking: [{ path: userPath, keys: [
    'env.anthropic_auth_token', 'env.Anthropic_Api_Key', 'env.claude_code_use_bedrock',
    'env.Claude_Code_Use_Vertex', 'env.ClAuDe_CoDe_UsE_FoUnDrY', 'env.Claude_Code_Use_Anthropic_Aws',
  ] }] });
});

test('Windows overrides an ordinary mixed-case base URL but blocks a managed one', async () => {
  const values = { Anthropic_Base_Url: 'https://synthetic.invalid' };
  const ordinary = await inspect(values);
  const managed = await inspect(values, 'win32', true);
  assert.deepEqual({ ordinary, managed }, {
    ordinary: { overrideBaseUrl: true, blocking: [] },
    managed: { overrideBaseUrl: false, blocking: [{ path: managedPath, keys: ['env.Anthropic_Base_Url'] }] },
  });
});

test('one conflicting Windows case variant is not hidden by a matching variant', async () => {
  assert.deepEqual(await inspect({ ANTHROPIC_AUTH_TOKEN: expected.ANTHROPIC_AUTH_TOKEN,
    anthropic_auth_token: 'synthetic-other-token' }), { overrideBaseUrl: false,
    blocking: [{ path: userPath, keys: ['env.anthropic_auth_token'] }] });
});

test('matching Windows aliases and ordinary uppercase behavior remain valid', async () => {
  assert.deepEqual(await inspect({ anthropic_auth_token: expected.ANTHROPIC_AUTH_TOKEN,
    Anthropic_Base_Url: expected.ANTHROPIC_BASE_URL }), { overrideBaseUrl: false, blocking: [] });
  assert.deepEqual(await inspect({ ANTHROPIC_API_KEY: 'synthetic-api-key',
    ANTHROPIC_AUTH_TOKEN: 'synthetic-other-token' }), { overrideBaseUrl: false,
    blocking: [{ path: userPath, keys: ['env.ANTHROPIC_AUTH_TOKEN', 'env.ANTHROPIC_API_KEY'] }] });
});

test('POSIX settings environment keys remain case-sensitive', async () => {
  for (const platform of ['linux', 'darwin']) {
    assert.deepEqual(await inspect({ anthropic_auth_token: 'synthetic-other-token',
      claude_code_use_bedrock: '1', ANTHROPIC_API_KEY: 'synthetic-api-key' }, platform), {
      overrideBaseUrl: false, blocking: [{ path: userPath, keys: ['env.ANTHROPIC_API_KEY'] }],
    });
  }
});
