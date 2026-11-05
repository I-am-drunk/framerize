// Version and conflict cases. Lookups and settings reads are synthetic; no network or client runs.
import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/compatibility/index.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const setup = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const origin = 'http://127.0.0.1:8788';
const native = '/home/me/.local/bin/claude';
const latestUrl = channel => `https://downloads.claude.ai/claude-code-releases/${channel}`;

function deps({ installed = [native], files = {}, releases = {}, answers = [], version = '2.1.300 (Claude Code)\n' } = {}) {
  const runs = [];
  const fetched = [];
  const out = [];
  const err = [];
  const present = new Set(installed);
  return { runs, fetched, out, err, deps: {
    platform: 'linux', env: { PATH: '/home/me/.local/bin:/bin' }, home: '/home/me', stateDir: '/synthetic/state',
    cwd: '/synthetic/project', target: { origin, localToken: 'synthetic-local-token' },
    io: { stdout: text => out.push(text), stderr: text => err.push(text) },
    probe: path => present.has(path), realPath: path => path,
    runner: { async run(command) { runs.push(command); return 0; },
      async capture(command) { return command.args[0] === '--version' ? version : null; } },
    fetch: async url => { fetched.push(url); return releases[url] ?? null; },
    read: async path => files[path.replaceAll('\\', '/')] ?? null,
    ...(answers ? { prompt: async () => answers.shift() ?? null } : {}),
  } };
}

test('a current client is reported current and no update is offered', async () => {
  const h = deps({ releases: { [latestUrl('latest')]: '2.1.300\n' }, answers: ['update'] });
  assert.equal(await setup.runClient(['claude', '--update'], h.deps), 0);
  assert.equal(h.runs.length, 0);
  assert.match(h.out.join(''), /Version 2\.1\.300 is the latest\./);
});

test('a failed lookup is unknown, not current, and still needs the typed update', async () => {
  const h = deps({ answers: ['no'] });
  assert.equal(await setup.runClient(['claude', '--update'], h.deps), setup.EXIT.needsConsent);
  assert.deepEqual(h.fetched, [latestUrl('latest')]);
  assert.match(h.out.join(''), /the latest version could not be checked/);
  assert.equal(h.runs.length, 0);
});

test('the setup offer asks about updates only for a client that is behind', async () => {
  const unknown = deps({ installed: [native, '/bin/codex'], answers: [] });
  const results = await setup.offerClients(unknown.deps);
  assert.deepEqual(results.map(result => result.version), ['unknown', 'unknown']);
  assert.equal(unknown.runs.length, 0);
});

test('a stable channel in Claude settings selects the stable release lookup', async () => {
  const h = deps({ files: { '/home/me/.claude/settings.json': '{"autoUpdatesChannel":"stable"}' },
    releases: { [latestUrl('stable')]: '2.1.300' } });
  assert.equal(await setup.runClient(['claude', '--update'], h.deps), 0);
  assert.deepEqual(h.fetched, [latestUrl('stable')]);
});

test('T3 semver order puts prereleases before their release', () => {
  assert.ok(setup.compareSemverVersions('2.1.9', '2.1.10') < 0);
  assert.ok(setup.compareSemverVersions('1.0.0-beta.2', '1.0.0') < 0);
  assert.ok(setup.compareSemverVersions('1.0.0-beta.10', '1.0.0-beta.2') > 0);
  assert.equal(setup.versionStatus('2.1.0', null, setup.compareSemverVersions).status, 'unknown');
});

test('a foreign user base URL is overridden per launch with --settings and no token in argv', async () => {
  const files = { '/home/me/.claude/settings.json': '{"env":{"ANTHROPIC_BASE_URL":"https://gateway.example"},"model":"opus"}' };
  const h = deps({ files });
  assert.equal(await setup.runClient(['claude', '--', '--model', 'claude-opus-5-5'], h.deps), 0);
  const [launch] = h.runs;
  assert.deepEqual(launch.args, ['--settings', `{"env":{"ANTHROPIC_BASE_URL":"${origin}/anthropic"}}`,
    '--model', 'claude-opus-5-5']);
  assert.ok(!launch.args.join(' ').includes('synthetic-local-token'));
  assert.equal(launch.env.ANTHROPIC_AUTH_TOKEN, 'synthetic-local-token');
});

for (const [name, path, settings, key] of [
  ['a user apiKeyHelper', '/home/me/.claude/settings.json', { apiKeyHelper: '~/bin/key.sh' }, 'apiKeyHelper'],
  ['a project credential', '/synthetic/project/.claude/settings.local.json', { env: { ANTHROPIC_AUTH_TOKEN: 'synthetic-other' } }, 'env.ANTHROPIC_AUTH_TOKEN'],
  ['a cloud provider switch', '/synthetic/project/.claude/settings.json', { env: { CLAUDE_CODE_USE_BEDROCK: '1' } }, 'env.CLAUDE_CODE_USE_BEDROCK'],
  ['a managed base URL', '/etc/claude-code/managed-settings.json', { env: { ANTHROPIC_BASE_URL: 'https://corp.example' } }, 'env.ANTHROPIC_BASE_URL'],
]) {
  test(`${name} blocks the launch before spawning and names only the key`, async () => {
    const h = deps({ files: { [path]: JSON.stringify(settings) } });
    assert.equal(await setup.runClient(['claude'], h.deps), setup.EXIT.failed);
    assert.equal(h.runs.length, 0);
    assert.match(h.err.join('').replaceAll('\\', '/'),
      new RegExp(`${path.replaceAll('.', '\\.')}: ${key.replaceAll('.', '\\.')}`));
    assert.ok(!/synthetic-other|corp\.example|key\.sh/.test(h.err.join('')));
  });
}

test('values the launch already sets and unreadable files do not block', async () => {
  const files = { '/home/me/.claude/settings.json': JSON.stringify({ env: {
    ANTHROPIC_BASE_URL: `${origin}/anthropic`, ANTHROPIC_AUTH_TOKEN: 'synthetic-local-token' } }),
  '/synthetic/project/.claude/settings.json': '{not json' };
  const h = deps({ files });
  assert.equal(await setup.runClient(['claude'], h.deps), 0);
  assert.deepEqual(h.runs[0].args, []);
});

test('cancelling a release lookup prevents a consented standalone update', async () => {
  const h = deps(), controller = new AbortController();
  h.deps.signal = controller.signal;
  h.deps.fetch = async () => { controller.abort(); return '2.1.301'; };
  await assert.rejects(setup.runClient(['claude', '--update', '--no-input'], h.deps), { name: 'AbortError' });
  assert.deepEqual(h.runs, []);
  assert.deepEqual(h.out, []);
  assert.deepEqual(h.err, []);
});

test('the standalone update confirmation receives cancellation', { timeout: 2000 }, async () => {
  const h = deps({ releases: { [latestUrl('latest')]: '2.1.301' } });
  const controller = new AbortController(), reached = Promise.withResolvers();
  let received, release;
  h.deps.signal = controller.signal;
  h.deps.prompt = (_question, signal) => new Promise((resolve, reject) => {
    received = signal;
    release = () => resolve('no');
    signal?.addEventListener('abort', () => reject(signal.reason), { once: true });
    reached.resolve();
  });
  const pending = setup.runClient(['claude', '--update'], h.deps);
  await reached.promise;
  controller.abort();
  release();
  await assert.rejects(pending, { name: 'AbortError' });
  assert.equal(received, controller.signal);
  assert.deepEqual(h.runs, []);
});
