// All client, package-manager, settings and network operations are injected.
import test from 'node:test';
import assert from 'node:assert/strict';
import { posix } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/index.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { offerClients } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

const cases = [
  ['claude', '/synthetic/home/.local/bin/claude'],
  ['claude', '/synthetic/home/.bun/bin/claude'],
  ['codex', '/synthetic/home/.codex/packages/standalone/current/bin/codex'],
  ['codex', '/synthetic/home/.bun/bin/codex'],
  ['claude', '/opt/synthetic-brew/bin/claude'],
  ['codex', '/opt/synthetic-brew/bin/codex'],
];

for (const [client, alias] of cases) {
  test(`setup leaves the Homebrew update manual for ${alias}`, async () => {
    const label = client === 'claude' ? 'Claude Code' : 'Codex';
    const cask = client === 'claude' ? 'claude-code' : 'codex';
    const brew = '/synthetic/bin/brew', npm = '/synthetic/bin/npm';
    const events = [], output = [], errors = [];
    const deps = {
      platform: 'darwin', env: { PATH: `${posix.dirname(alias)}:/synthetic/bin` },
      home: '/synthetic/home', stateDir: '/synthetic/state', cwd: '/synthetic/project',
      target: { origin: 'http://127.0.0.1:8788' },
      io: { stdout: text => output.push(text), stderr: text => errors.push(text) },
      probe: path => [alias, brew, npm].includes(path),
      realPath: path => {
        assert.equal(path, alias);
        return `/opt/synthetic-brew/Caskroom/${cask}/2.1.0/${client}`;
      },
      read: async () => null,
      fetch: async url => { events.push({ kind: 'fetch', url }); return '2.2.0'; },
      prompt: async question => { events.push({ kind: 'prompt', question }); return 'update'; },
      runner: {
        async capture({ command, args }) {
          events.push({ kind: 'capture', command, args });
          if (command === alias) return '2.1.0';
          if (command === brew) return JSON.stringify({ casks: [{ version: '2.2.0' }] });
          // A wrong owner still gets a newer version, exposing any attempted update.
          if (command === npm) return JSON.stringify('2.2.0');
          throw new Error(`Unexpected probe: ${command}`);
        },
        async run({ command, args }) {
          events.push({ kind: 'run', command, args });
          return 0;
        },
      },
    };
    const results = await offerClients(deps, [client]);
    assert.deepEqual({ results, events, output: output.join(''), errors }, {
      results: [{ client, state: 'present', version: 'behind' }],
      events: [
        { kind: 'capture', command: alias, args: ['--version'] },
        { kind: 'capture', command: brew, args: ['info', '--json=v2', '--cask', cask] },
      ],
      output: `${label}: ${alias}\nVersion 2.1.0; 2.2.0 is available.\n`
        + `To update ${label}: brew upgrade --cask ${cask}\n`,
      errors: [],
    });
  });
}
