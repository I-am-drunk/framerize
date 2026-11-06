import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { mkdtemp, readFile, rm, writeFile, mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/compatibility/index.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const setup = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const { runClient, EXIT } = setup;
const token = 'synthetic-local-token';
const origin = 'http://127.0.0.1:8788';

const installerScript = (url, shell) => [
  'installer=$(mktemp "${TMPDIR:-/tmp}/framerize-install.XXXXXX") || exit "$?"',
  'trap \'rm -f "$installer"\' 0',
  'trap \'exit 1\' HUP INT TERM',
  `curl -fsSL ${url} > "$installer" || exit "$?"`,
  `${shell} < "$installer"`,
].join('\n');

/** Stub boundaries only: no real PATH lookup, prompt or process ever runs. */
function harness({ platform = 'linux', installed = [], afterInstall = [], answers = [], exit = 0,
  localToken = token, env = {}, home = '/synthetic/home', stateDir = '/synthetic/state',
  releases = {}, files = {}, version = '2.1.300 (Claude Code)\n' } = {}) {
  const present = new Set(installed);
  const calls = [];
  const probes = [];
  const fetched = [];
  const questions = [];
  const out = [];
  const err = [];
  const deps = { platform, env: { PATH: platform === 'win32' ? 'C:\\bin' : '/bin', ...env }, home, stateDir,
    target: { origin, ...(localToken ? { localToken } : {}) },
    io: { stdout: text => out.push(text), stderr: text => err.push(text) },
    probe: candidate => present.has(candidate),
    realPath: path => path,
    runner: {
      async run(command, runEnv) {
        calls.push({ ...command, runEnv });
        if (/install\.(sh|ps1)/.test(command.display)) afterInstall.forEach(path => present.add(path));
        return exit;
      },
      // Captures are read-only probes (--version, npm view, brew info); they never count as runs.
      async capture(command) {
        probes.push([command.command, ...command.args].join(' '));
        if (command.args[0] === '--version') return version;
        return Object.entries(releases).find(([prefix]) => probes.at(-1).includes(prefix))?.[1] ?? null;
      },
    },
    fetch: async url => { fetched.push(url); return releases[url] ?? null; },
    read: async path => files[path] ?? null,
    cwd: '/synthetic/project',
    ...(answers ? { prompt: async question => { questions.push(question); return answers.shift() ?? null; } } : {}),
  };
  return { deps, calls, probes, fetched, questions, out, err, text: () => out.join(''), errors: () => err.join('') };
}

test('Claude launches against the Anthropic route with the local bearer token', async () => {
  const h = harness({ installed: ['/bin/claude'], env: { ANTHROPIC_API_KEY: 'synthetic-console-key' } });
  const code = await runClient(['claude', '--', '--model', 'claude-opus-5-5', '-p', 'hi'], h.deps);
  assert.equal(code, 0);
  assert.equal(h.calls.length, 1);
  const [launch] = h.calls;
  assert.equal(launch.command, '/bin/claude');
  assert.deepEqual(launch.args, ['--model', 'claude-opus-5-5', '-p', 'hi']);
  assert.deepEqual(launch.env, { ANTHROPIC_BASE_URL: `${origin}/anthropic`, ANTHROPIC_AUTH_TOKEN: token });
  assert.equal(launch.runEnv.ANTHROPIC_API_KEY, undefined);
  assert.equal(h.questions.length, 0);
});

for (const key of ['CLAUDE_CODE_USE_BEDROCK', 'CLAUDE_CODE_USE_VERTEX',
  'CLAUDE_CODE_USE_FOUNDRY', 'CLAUDE_CODE_USE_ANTHROPIC_AWS']) {
  test(`Claude launch clears inherited ${key} for the Remote session`, async () => {
    const h = harness({ installed: ['/bin/claude'], env: { [key]: '1', AWS_REGION: 'synthetic-region' } });
    assert.equal(await runClient(['claude'], h.deps), 0);
    assert.equal(h.calls.length, 1);
    assert.equal(h.calls[0].runEnv[key], undefined);
    assert.equal(h.calls[0].runEnv.AWS_REGION, 'synthetic-region');
    assert.equal(h.calls[0].env.ANTHROPIC_BASE_URL, `${origin}/anthropic`);
    assert.equal(h.deps.env[key], '1', 'the parent environment remains unchanged');
  });
}

test('Claude without a local token still sends a placeholder instead of a saved login', async () => {
  const h = harness({ installed: ['/bin/claude'], localToken: null });
  assert.equal(await runClient(['claude'], h.deps), 0);
  assert.equal(h.calls[0].env.ANTHROPIC_AUTH_TOKEN, 'framerize-no-local-token');
});

test('Codex launches with a responses provider and keeps the token out of arguments', async () => {
  const h = harness({ installed: ['/bin/codex'] });
  assert.equal(await runClient(['codex', '--', '--model', 'gpt-6.1-sol', 'exec', 'hi'], h.deps), 0);
  const [launch] = h.calls;
  assert.deepEqual(launch.args.slice(0, 4), ['-c',
    `model_providers.framerize={name="Framerize Remote",base_url="${origin}/openai/v1",wire_api="responses",env_key="FRM_LOCAL_TOKEN"}`,
    '-c', 'model_provider="framerize"']);
  assert.deepEqual(launch.args.slice(4), ['--model', 'gpt-6.1-sol', 'exec', 'hi']);
  assert.deepEqual(launch.env, { FRM_LOCAL_TOKEN: token });
  assert.ok(!launch.args.join(' ').includes(token));
  assert.ok(!launch.display.includes(token));
});

test('Codex without a local token omits env_key and adds no model default', async () => {
  const h = harness({ installed: ['/bin/codex'], localToken: null });
  assert.equal(await runClient(['codex'], h.deps), 0);
  assert.ok(!h.calls[0].args[1].includes('env_key'));
  assert.equal(h.calls[0].args.length, 4);
  assert.equal(h.calls[0].env, undefined);
});

test('a missing client without consent and without a terminal installs nothing', async () => {
  const h = harness({ answers: null });
  assert.equal(await runClient(['claude'], h.deps), EXIT.needsConsent);
  assert.equal(h.calls.length, 0);
  assert.match(h.text(), /Download https:\/\/claude\.ai\/install\.sh, then run bash/);
  assert.match(h.errors(), /--install or FRM_CLIENT_INSTALL=1/);
});

test('--no-input never prompts even when a terminal is present', async () => {
  const h = harness({ answers: ['install'] });
  assert.equal(await runClient(['codex', '--no-input'], h.deps), EXIT.needsConsent);
  assert.equal(h.questions.length, 0);
  assert.equal(h.calls.length, 0);
});

test('a terminal decline installs nothing', async () => {
  const h = harness({ answers: ['y'] });
  assert.equal(await runClient(['claude'], h.deps), EXIT.needsConsent);
  assert.equal(h.questions.length, 1);
  assert.equal(h.calls.length, 0);
});

test('typing install runs the official installer, then launches the new client', async () => {
  const h = harness({ answers: ['install'], afterInstall: ['/bin/claude'] });
  assert.equal(await runClient(['claude'], h.deps), 0);
  assert.deepEqual(h.calls.map(call => call.command), ['/bin/sh', '/bin/claude']);
  assert.deepEqual(h.calls[0].args, ['-c', installerScript('https://claude.ai/install.sh', 'bash')]);
});

test('agent install consent comes from FRM_CLIENT_INSTALL without a prompt', async () => {
  const h = harness({ answers: null, env: { FRM_CLIENT_INSTALL: '1' }, afterInstall: ['/bin/codex'] });
  assert.equal(await runClient(['codex'], h.deps), 0);
  assert.deepEqual(h.calls[0].args, ['-c', installerScript('https://chatgpt.com/codex/install.sh', 'sh')]);
  assert.equal(h.calls[1].command, '/bin/codex');
});

test('a failed installer stops before launch', async () => {
  const h = harness({ answers: null, exit: 7, env: { FRM_CLIENT_INSTALL: '1' } });
  assert.equal(await runClient(['claude'], h.deps), EXIT.failed);
  assert.equal(h.calls.length, 1);
  assert.match(h.errors(), /exited with code 7/);
});

test('an installer that leaves the client off PATH reports a new terminal', async () => {
  const h = harness({ answers: null, env: { FRM_CLIENT_INSTALL: '1' } });
  assert.equal(await runClient(['claude'], h.deps), EXIT.failed);
  assert.match(h.errors(), /not on PATH yet/);
});

test('Windows uses the PowerShell installer and PATHEXT lookup', async () => {
  const h = harness({ platform: 'win32', answers: null, env: { FRM_CLIENT_INSTALL: '1', PATHEXT: '.EXE;.CMD' },
    afterInstall: ['C:\\bin\\claude.exe'] });
  assert.equal(await runClient(['claude'], h.deps), 0);
  assert.equal(h.calls[0].command, 'powershell.exe');
  assert.equal(h.calls[0].args.at(-1), 'irm https://claude.ai/install.ps1 | iex');
  assert.equal(h.calls[1].command, 'C:\\bin\\claude.exe');
});

test('an unsupported OS gets the documentation link and no installer', async () => {
  const h = harness({ platform: 'freebsd', env: { FRM_CLIENT_INSTALL: '1' } });
  assert.equal(await runClient(['codex'], h.deps), EXIT.failed);
  assert.equal(h.calls.length, 0);
  assert.match(h.errors(), /freebsd from https:\/\/developers\.openai\.com\/codex\/cli/);
});

const npmClaude = '/synthetic/prefix/lib/node_modules/@anthropic-ai/claude-code/cli.js';
const newer = { 'https://downloads.claude.ai/claude-code-releases/latest': '2.1.400\n' };

test('an installed client is never updated by install consent alone', async () => {
  const h = harness({ installed: ['/bin/claude'], env: { FRM_CLIENT_INSTALL: '1', FRM_CLIENT_UPDATE: '1' } });
  assert.equal(await runClient(['claude'], h.deps), 0);
  assert.equal(h.calls.length, 1);
  assert.equal(h.calls[0].command, '/bin/claude');
});

test('--update in a terminal needs the typed word and a decline runs nothing', async () => {
  const h = harness({ installed: ['/home/me/.local/bin/claude'], answers: ['yes'], releases: newer,
    env: { PATH: '/home/me/.local/bin' } });
  assert.equal(await runClient(['claude', '--update'], h.deps), EXIT.needsConsent);
  assert.match(h.questions[0], /Type update/);
  assert.equal(h.calls.length, 0);
});

test('a confirmed native Claude update runs claude update', async () => {
  const h = harness({ installed: ['/home/me/.local/bin/claude'], answers: ['update'], releases: newer,
    env: { PATH: '/home/me/.local/bin' } });
  assert.equal(await runClient(['claude', '--update'], h.deps), 0);
  assert.deepEqual(h.calls.map(call => [call.command, ...call.args]), [['/home/me/.local/bin/claude', 'update']]);
});

test('an npm-owned Claude update targets its own prefix', async () => {
  const h = harness({ installed: ['/synthetic/prefix/bin/claude', '/synthetic/prefix/bin/npm'], answers: null,
    env: { PATH: '/synthetic/prefix/bin' }, releases: { 'npm view --global --prefix /synthetic/prefix': '"2.1.400"' } });
  h.deps.realPath = path => path.endsWith('claude') ? npmClaude : path;
  assert.equal(await runClient(['claude', '--update', '--no-input'], h.deps), 0);
  assert.ok(h.probes.includes('/synthetic/prefix/bin/npm view --global --prefix /synthetic/prefix --json @anthropic-ai/claude-code@latest version'));
  assert.deepEqual(h.calls[0].args, ['install', '-g', '--prefix', '/synthetic/prefix',
    '--allow-scripts=@anthropic-ai/claude-code', '@anthropic-ai/claude-code@latest']);
});

test('a Homebrew cask update is shown and never run', async () => {
  const h = harness({ platform: 'darwin', installed: ['/opt/homebrew/bin/codex'], answers: ['update'],
    env: { PATH: '/opt/homebrew/bin' } });
  h.deps.realPath = () => '/opt/homebrew/Caskroom/codex/0.170.0/codex';
  assert.equal(await runClient(['codex', '--update'], h.deps), 0);
  assert.equal(h.calls.length, 0);
  assert.match(h.text(), /brew upgrade --cask codex/);
});

test('the setup offer installs a missing client only after the typed word and starts nothing', async () => {
  const h = harness({ installed: ['/home/me/.local/bin/claude'], answers: ['no', 'install'], releases: newer,
    afterInstall: ['/bin/codex'], env: { PATH: '/home/me/.local/bin:/bin' } });
  const results = await setup.offerClients(h.deps);
  assert.deepEqual(results, [{ client: 'claude', state: 'present', version: 'behind' },
    { client: 'codex', state: 'installed' }]);
  assert.deepEqual(h.calls.map(call => call.args.at(-1)), [installerScript('https://chatgpt.com/codex/install.sh', 'sh')]);
  assert.match(h.questions[0], /Update Claude Code with \/home\/me\/\.local\/bin\/claude update\? Type update/);
  assert.match(h.questions[1], /Install Codex\? Type install/);
});

test('the setup offer without a terminal reports state and runs nothing but version probes', async () => {
  const h = harness({ answers: null, installed: ['/bin/codex'], env: { FRM_CLIENT_INSTALL: '1' } });
  const results = await setup.offerClients(h.deps);
  assert.deepEqual(results.map(result => result.state), ['declined', 'present']);
  assert.equal(h.calls.length, 0);
  assert.match(h.text(), /No terminal: nothing was installed or updated/);
});

test('--persist keeps unrelated Claude settings and --undo removes only Framerize values', async context => {
  const root = await mkdtemp(join(tmpdir(), 'frm-client-settings-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const config = join(root, 'claude');
  const state = join(root, 'state');
  await mkdir(config);
  const original = { model: 'opus', env: { DISABLE_AUTOUPDATER: '1' }, permissions: { allow: ['Bash(ls)'] } };
  await writeFile(join(config, 'settings.json'), JSON.stringify(original));
  const h = harness({ env: { CLAUDE_CONFIG_DIR: config }, stateDir: state });
  assert.equal(await runClient(['claude', '--persist'], h.deps), 0);
  const saved = JSON.parse(await readFile(join(config, 'settings.json'), 'utf8'));
  assert.deepEqual(saved.env, { DISABLE_AUTOUPDATER: '1', ANTHROPIC_BASE_URL: `${origin}/anthropic`, ANTHROPIC_AUTH_TOKEN: token });
  assert.ok(!(await readFile(join(state, 'claude-settings.json'), 'utf8')).includes(token));
  assert.equal(await runClient(['claude', '--undo'], h.deps), 0);
  assert.deepEqual(JSON.parse(await readFile(join(config, 'settings.json'), 'utf8')), original);
  assert.equal(await runClient(['claude', '--undo'], h.deps), 0);
  assert.match(h.text(), /has not written Claude settings/);
  assert.equal(h.calls.length, 0);
});

test('--persist refuses a gateway value it did not write and undo keeps a later edit', async context => {
  const root = await mkdtemp(join(tmpdir(), 'frm-client-foreign-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const config = join(root, 'claude');
  await mkdir(config);
  const foreign = { env: { ANTHROPIC_BASE_URL: 'https://gateway.example' } };
  await writeFile(join(config, 'settings.json'), JSON.stringify(foreign));
  const h = harness({ env: { CLAUDE_CONFIG_DIR: config }, stateDir: join(root, 'state') });
  assert.equal(await runClient(['claude', '--persist'], h.deps), EXIT.failed);
  assert.deepEqual(JSON.parse(await readFile(join(config, 'settings.json'), 'utf8')), foreign);
  await writeFile(join(config, 'settings.json'), '{}');
  assert.equal(await runClient(['claude', '--persist'], h.deps), 0);
  const edited = { env: { ANTHROPIC_BASE_URL: 'https://edited.example', ANTHROPIC_AUTH_TOKEN: token } };
  await writeFile(join(config, 'settings.json'), JSON.stringify(edited));
  assert.equal(await runClient(['claude', '--undo'], h.deps), 0);
  assert.deepEqual(JSON.parse(await readFile(join(config, 'settings.json'), 'utf8')), edited);
  assert.match(h.text(), /changed since Framerize wrote them/);
});
