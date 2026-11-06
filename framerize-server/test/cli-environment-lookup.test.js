import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const source = fileURLToPath(new URL('../src/', import.meta.url));
const replacements = {
  'setup.ts': 'export const runSetupCommand = (args, deps) => globalThis.__serverEnvCaller(args, deps);',
  'server.ts': 'export const createServer = () => { throw new Error("unexpected server"); };',
  'backend.ts': 'export const createConfiguredProvider = () => { throw new Error("unexpected provider"); };',
};
const compiled = await build({ stdin: { resolveDir: source,
  contents: "export { run } from './cli.ts'; export { configuredSettings } from './settings.ts';" },
  bundle: true, platform: 'node', format: 'esm', write: false,
  plugins: [{ name: 'isolate-setup-effects', setup(builder) {
    builder.onLoad({ filter: /(setup|server|backend)\.ts$/ }, args =>
      dirname(args.path) === resolve(source)
        ? { contents: replacements[basename(args.path)], loader: 'ts' } : undefined);
  } }],
});
const { run, configuredSettings } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const insensitive = values => new Proxy(values, { get(target, name) {
  if (typeof name !== 'string') return Reflect.get(target, name);
  const key = Object.keys(target).find(key => key.toLowerCase() === name.toLowerCase());
  return key === undefined ? Reflect.get(target, name) : target[key];
} });

for (const command of ['up', 'pair', 'status', 'service', 'uppercase control']) {
  test(`CLI preserves environment lookups for ${command}`, async context => {
    const home = await mkdtemp(join(tmpdir(), 'frm-server-env-caller-'));
    const mixed = command !== 'uppercase control';
    const values = mixed ? { HoMe: home, Framerize_Server_Host: '127.0.0.2',
      Synthetic_Token: 'fixture-token', pAtH: 'fixture-search-path' }
      : { HOME: home, FRAMERIZE_SERVER_HOST: '127.0.0.2',
        SYNTHETIC_TOKEN: 'fixture-token', PATH: 'fixture-search-path' };
    const env = insensitive(values);
    const events = new EventEmitter();
    events.stdout = events.stderr = { write() {} };
    const before = ['SIGINT', 'SIGTERM'].map(name => events.listeners(name));
    const observed = [];
    const prior = globalThis.__serverEnvCaller;
    context.after(async () => {
      if (prior === undefined) delete globalThis.__serverEnvCaller;
      else globalThis.__serverEnvCaller = prior;
      await rm(home, { recursive: true, force: true });
    });
    globalThis.__serverEnvCaller = async (args, deps) => {
      const loaded = await configuredSettings(undefined, { accountsFile: join(home, 'accounts.json') }, deps.env);
      observed.push({ command: args[0], host: loaded.config.host,
        token: loaded.env.SYNTHETIC_TOKEN, search: loaded.env.PATH,
        entries: Object.fromEntries(Object.entries(loaded.env)) });
      return 0;
    };
    const selected = mixed ? command : 'status';
    assert.equal(await run([selected], { process: events, env }), 0);
    assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => events.listeners(name)), before);
    assert.deepEqual(values, { ...(mixed ? { HoMe: home, Framerize_Server_Host: '127.0.0.2',
      Synthetic_Token: 'fixture-token', pAtH: 'fixture-search-path' }
      : { HOME: home, FRAMERIZE_SERVER_HOST: '127.0.0.2',
        SYNTHETIC_TOKEN: 'fixture-token', PATH: 'fixture-search-path' }) });
    await rm(home, { recursive: true, force: true });
    assert.equal(observed.length, 1);
    assert.deepEqual(observed[0], { command: selected, host: '127.0.0.2',
      token: 'fixture-token', search: 'fixture-search-path', entries: values });
  });
}
