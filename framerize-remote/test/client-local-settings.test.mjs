import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const built = await build({ entryPoints: [fileURLToPath(new URL('../src/compatibility/index.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const { runClient } = await import(`data:text/javascript;base64,${Buffer.from(built.outputFiles[0].contents).toString('base64')}`);
const root = '/synthetic/repository', cwd = `${root}/packages/app`;
const local = `${root}/.claude/settings.local.json`, nested = `${cwd}/.claude/settings.local.json`;
const uid = process.getuid?.();
const posix = { skip: uid === undefined && 'Repository ownership requires POSIX user IDs' };

function fixture({ files = {}, platform = 'linux', home = '/synthetic/home', gitRoot = root,
  foreign, absent = [], canonical = path => path } = {}) {
  const runs = [], errors = [], reads = [], probes = [];
  const deps = { platform, home, cwd, env: { PATH: '/synthetic/bin' }, stateDir: '/synthetic/state',
    target: { origin: 'http://127.0.0.1:8788', localToken: 'synthetic-local' },
    io: { stdout() {}, stderr(text) { errors.push(text.replaceAll('\\', '/')); } },
    probe: path => /(?:^|[/\\])claude(?:\.(?:exe|cmd))?$/.test(path), realPath: canonical,
    read: async path => { path = path.replaceAll('\\', '/'); reads.push(path); return files[path] ?? null; },
    fileOwner: async path => absent.includes(path) ? null : path === foreign ? uid + 1 : uid,
    runner: { async run(command) { runs.push(command); return 0; },
      async capture(command) {
        probes.push(command);
        assert.equal(command.command, 'git');
        assert.deepEqual(command.args, ['-C', cwd, 'rev-parse', '--show-toplevel']);
        return gitRoot === null ? null : `${gitRoot}\n`;
      } },
  };
  return { deps, runs, errors, reads, probes };
}

test('a repository-root local credential blocks a nested launch', posix, async () => {
  const h = fixture({ files: { [local]: '{"env":{"ANTHROPIC_AUTH_TOKEN":"private-canary"}}' } });
  assert.equal(await runClient(['claude'], h.deps), 1);
  assert.equal(h.runs.length, 0);
  assert.ok(h.errors.join('').includes(`${local}: env.ANTHROPIC_AUTH_TOKEN`));
  assert.ok(!h.errors.join('').includes('private-canary'));
});

test('a repository-root local URL gets the existing token-free override', posix, async () => {
  const h = fixture({ files: { [local]: '{"env":{"ANTHROPIC_BASE_URL":"https://other.example"}}' } });
  assert.equal(await runClient(['claude'], h.deps), 0);
  assert.deepEqual(h.runs[0].args, ['--settings', '{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8788/anthropic"}}']);
  assert.equal(h.runs[0].env.ANTHROPIC_AUTH_TOKEN, 'synthetic-local');
  assert.ok(!h.runs[0].args.join(' ').includes('synthetic-local'));
});

test('a nested local file is ignored when Claude uses the root local file', posix, async () => {
  const h = fixture({ files: { [nested]: '{"apiKeyHelper":"private-canary"}' } });
  assert.equal(await runClient(['claude'], h.deps), 0);
  assert.equal(h.runs.length, 1);
  assert.ok(h.reads.includes(local));
  assert.ok(!h.reads.includes(nested));
});

for (const [name, options] of [
  ['Windows', { platform: 'win32' }],
  ['outside Git', { gitRoot: null }],
  ['a home-directory repository', { home: root }],
  ['a home-directory symlink', { home: '/synthetic/home', canonical: path => path === root ? '/synthetic/home' : path }],
  ['an unowned root', { foreign: root }],
  ['an unowned Git entry', { foreign: `${root}/.git` }],
  ['an unowned settings directory', { foreign: `${root}/.claude` }],
]) {
  test(`${name} keeps the local file beside the shared project file`, async () => {
    const h = fixture({ ...options, files: { [nested]: '{"apiKeyHelper":"private-canary"}' } });
    assert.equal(await runClient(['claude'], h.deps), 1);
    assert.equal(h.runs.length, 0);
    assert.ok(h.errors.join('').includes(nested));
    assert.ok(!h.errors.join('').includes('private-canary'));
    if (options.platform === 'win32') assert.equal(h.probes.length, 0);
  });
}

test('shared settings still come from the working directory', posix, async () => {
  const path = `${cwd}/.claude/settings.json`;
  const h = fixture({ files: { [path]: '{"apiKeyHelper":"private-canary"}' } });
  assert.equal(await runClient(['claude'], h.deps), 1);
  assert.ok(h.errors.join('').includes(path));
  assert.ok(!h.reads.includes(`${root}/.claude/settings.json`));
});

test('an absent root settings directory still selects the root local path', posix, async () => {
  const h = fixture({ absent: [`${root}/.claude`], files: { [nested]: '{"apiKeyHelper":"private-canary"}' } });
  assert.equal(await runClient(['claude'], h.deps), 0);
  assert.ok(h.reads.includes(local));
  assert.ok(!h.reads.includes(nested));
});
