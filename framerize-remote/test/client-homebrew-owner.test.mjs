import test from 'node:test';
import assert from 'node:assert/strict';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/index.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { runClient } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

function fixture(executable, realPath) {
  const events = [], output = [], errors = [];
  const deps = { platform: 'darwin', env: { PATH: dirname(executable) },
    home: '/synthetic/home', stateDir: '/synthetic/state', cwd: '/synthetic/project',
    target: { origin: 'http://127.0.0.1:8788' },
    io: { stdout: text => output.push(text), stderr: text => errors.push(text) },
    probe: path => path.replaceAll('\\', '/') === executable,
    realPath: () => realPath, read: async () => null,
    fetch: async () => { events.push(['fetch']); return '2.2.0'; },
    runner: {
      async capture(command) { events.push(['capture', command]); return '2.1.0'; },
      async run(command) { events.push(['run', command]); return 0; },
    },
  };
  return { deps, events, output, errors };
}

for (const [client, alias] of [
  ['claude', '/synthetic/home/.local/bin/claude'],
  ['claude', '/synthetic/home/.bun/bin/claude'],
  ['claude', '/usr/local/bin/claude'],
  ['codex', '/synthetic/home/.codex/packages/standalone/current/bin/codex'],
  ['codex', '/synthetic/home/.bun/bin/codex'],
  ['codex', '/usr/local/bin/codex'],
]) {
  test(`a Homebrew ${client} behind ${alias} stays under Homebrew control`, async () => {
    const cask = client === 'claude' ? 'claude-code' : 'codex';
    const h = fixture(alias, `/opt/synthetic-brew/Caskroom/${cask}/2.1.0/${client}`);
    assert.equal(await runClient([client, '--update'], h.deps), 0);
    assert.deepEqual(h.events, [], 'a manual update must not probe or execute another updater');
    assert.deepEqual(h.errors, []);
    assert.equal(h.output.join(''), `${client === 'claude' ? 'Claude Code' : 'Codex'} at ${alias} is updated outside Framerize: brew upgrade --cask ${cask}\n`);
  });
}

for (const [client, alias, target, updater] of [
  ['claude', '/synthetic/home/.local/bin/claude', '/synthetic/home/.local/share/claude/versions/2.1.0', 'native'],
  ['codex', '/synthetic/home/.codex/packages/standalone/current/bin/codex', '/synthetic/home/.codex/packages/standalone/2.1.0/bin/codex', 'native'],
  ['claude', '/synthetic/home/.bun/bin/claude', '/synthetic/home/.bun/install/global/node_modules/@anthropic-ai/claude-code/cli.js', 'bun'],
  ['codex', '/synthetic/home/.bun/bin/codex', '/synthetic/home/.bun/install/global/node_modules/@openai/codex/bin/codex.js', 'bun'],
]) {
  test(`a real ${updater} ${client} still uses its updater`, async () => {
    const h = fixture(alias, target);
    assert.equal(await runClient([client, '--update'], h.deps), 0);
    const runs = h.events.filter(([kind]) => kind === 'run').map(([, command]) => command);
    assert.equal(runs.length, 1);
    assert.equal(runs[0].command, updater === 'native' ? alias : 'bun');
    assert.deepEqual(runs[0].args, updater === 'native' ? ['update'] :
      ['i', '-g', `${client === 'claude' ? '@anthropic-ai/claude-code' : '@openai/codex'}@latest`]);
    assert.deepEqual(h.errors, []);
  });
}
