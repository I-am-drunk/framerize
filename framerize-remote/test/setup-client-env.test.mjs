import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const clients = `export const offers = [];
  export const CLIENT_HELP = '';
  const unused = () => { throw Error('Unexpected process command'); };
  export const nodeRunner = () => ({ run: unused, capture: unused });
  export async function offerClients(deps) { offers.push(deps.target); }
  export function parseClientOptions() { throw Error('Unexpected client command'); }
  export function runClient() { throw Error('Unexpected client command'); }`;
const runtime = `export class StartupCleanupError extends Error {}
  export class StartupReadinessError extends Error {}
  export function runCheck() { throw Error('Unexpected readiness check'); }
  export function startServe() { throw Error('Unexpected listener'); }`;

const result = await build({ stdin: {
  resolveDir: fileURLToPath(new URL('../', import.meta.url)),
  contents: `export { run } from './src/cli.ts';
    export { offers } from './src/compatibility/index.ts';` },
  bundle: true, platform: 'node', format: 'esm', write: false,
  plugins: [{ name: 'synthetic-client-boundaries', setup(plugin) {
    plugin.onLoad({ filter: /[\\/]compatibility[\\/]index\.ts$/ }, () => ({ contents: clients }));
    plugin.onLoad({ filter: /[\\/]cmdRun\.ts$/ }, () => ({ contents: runtime }));
  } }],
});
const { run, offers } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].contents).toString('base64')}`);

const cases = [
  ['host flag overrides an invalid environment host', { FRM_HOST: 'bad/host' },
    ['--host', 'localhost', '--port', '4321'], 'http://localhost:4321'],
  ['port flag overrides an invalid environment port', { FRM_PORT: 'bad-port' },
    ['--port', '4321'], 'http://127.0.0.1:4321'],
  ['flags override valid conflicting environment values', { FRM_HOST: '127.0.0.2', FRM_PORT: '5432' },
    ['--host', 'localhost', '--port', '4321'], 'http://localhost:4321'],
  ['environment values still apply without flags', { FRM_HOST: '127.0.0.2', FRM_PORT: '5432' },
    [], 'http://127.0.0.2:5432'],
];

for (const [name, environment, flags, expected] of cases) {
  test(`setup client offer: ${name}`, async context => {
    const root = await mkdtemp(join(tmpdir(), 'setup-client-env-'));
    const relevant = key => /^(FRM_|FRAMERIZE_REMOTE_)/.test(key) || ['HOME', 'USERPROFILE'].includes(key);
    const previous = Object.fromEntries(Object.entries(process.env).filter(([key]) => relevant(key)));
    context.after(async () => {
      for (const key of Object.keys(process.env).filter(relevant)) delete process.env[key];
      Object.assign(process.env, previous);
      await rm(root, { recursive: true, force: true });
    });
    for (const key of Object.keys(process.env).filter(relevant)) delete process.env[key];
    Object.assign(process.env, { HOME: root, USERPROFILE: root, ...environment });
    offers.length = 0;
    const path = join(root, 'remote.json');
    const output = [];
    const io = { stdout: text => output.push(text), stderr: text => output.push(text), ask: async () => 'yes' };
    assert.equal(await run(['setup', '--config', path, '--remote', 'http://127.0.0.1:1234', ...flags], io), 0);
    assert.deepEqual(offers, [{ origin: expected }]);
    const saved = JSON.parse(await readFile(path, 'utf8'));
    assert.equal(`http://${saved.host}:${saved.port}`, expected);
    for (const [key, value] of Object.entries(environment)) assert.equal(process.env[key], value);
    assert.match(output.join(''), /Start a client with frm client/);
  });
}
