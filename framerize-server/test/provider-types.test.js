import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, cpSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

test('runtime declarations work without workspace packages and reject invalid contracts', { timeout: 30000 }, (context) => {
  const root = mkdtempSync(path.join(tmpdir(), 'provider types '));
  context.after(() => rmSync(root, { recursive: true, force: true }));
  const repository = fileURLToPath(new URL('../../', import.meta.url));
  const compiled = fileURLToPath(new URL('../dist/', import.meta.url));
  const dist = path.join(root, 'dist');
  mkdirSync(dist);
  for (const name of ['provider', 'core', 'server', 'logger', 'cli', 'config', 'framer-auth', 'framer-backend', 'account-backend', 'accounts']) {
    copyFileSync(path.join(compiled, `${name}.d.ts`), path.join(dist, `${name}.d.ts`));
  }
  cpSync(path.join(compiled, 'core-types'), path.join(dist, 'core-types'), { recursive: true });
  writeFileSync(path.join(root, 'package.json'), '{"type":"module"}');
  const valid = `import { createProvider, type ProviderOptions, type ProviderRequest } from './dist/provider.js';
import { createInstance, type InferenceBackend } from './dist/core.js';
import { createServer, type ServerProvider } from './dist/server.js';
import { createLogger, type LoggerOptions } from './dist/logger.js';
import { parseArgs, type CliDependencies } from './dist/cli.js';
import { createFramerAuth, type FramerAuthOptions } from './dist/framer-auth.js';
import { createFramerBackend, type FramerMinter } from './dist/framer-backend.js';
import { createAccountBackend, type AccountBackendOptions } from './dist/account-backend.js';
import { readAccountSession, type AccountSessionReference } from './dist/accounts.js';
const authOptions: FramerAuthOptions = { sessionCookie: 'synthetic' };
const auth = createFramerAuth(authOptions);
const castle: FramerMinter = { async mint({ signal }) { return { token: 'synthetic' }; }, async close() {} };
const framer: InferenceBackend = createFramerBackend({ projectId: 'project', auth, castle });
const accountOptions: AccountBackendOptions = { accountsFile: 'accounts.json', createMinter: () => castle };
const accountBackend: Promise<InferenceBackend> = createAccountBackend(accountOptions);
const reference: AccountSessionReference = { env: 'SYNTHETIC_SESSION' };
const session: Promise<string> = readAccountSession(reference, {});
const authClosed: Promise<void> = auth.close();
const syncProvider: ServerProvider = { ready: () => true,
  forward: () => ({ status: 200, headers: {}, body: [Buffer.from('{}')] }) };
const logger = createLogger({ level: 'info', stream: { write(text) {} } });
createServer({}, { provider: syncProvider, logger });
const args = parseArgs(['check']);
const dependencies: CliDependencies = { logger, createProvider: () => syncProvider };
const options: ProviderOptions = { upstream: 'http://127.0.0.1',
  credential: async ({ signal }) => { signal.throwIfAborted(); return 'synthetic'; } };
const provider = createProvider(options);
const backend: InferenceBackend = provider;
const app = createInstance({ backend, token: 'synthetic' });
const request: ProviderRequest = { path: '/v1/responses',
  body: (async function* () { yield Buffer.from('{}'); })() };
provider.forward(request);
provider.ready();
provider.ready({ signal: new AbortController().signal });
const closed: Promise<void> = provider.close();`;
  const check = (extra = '') => {
    writeFileSync(path.join(root, 'consumer.mts'), `${valid}\n${extra}\n`);
    const result = spawnSync(process.execPath, [path.join(repository, 'node_modules/typescript/bin/tsc'),
      '--noEmit', '--strict', '--noUncheckedIndexedAccess', '--exactOptionalPropertyTypes',
      '--target', 'ES2022', '--module', 'NodeNext', '--types', 'node',
      '--typeRoots', path.join(repository, 'node_modules/@types'), 'consumer.mts'],
    { cwd: root, encoding: 'utf8', timeout: 10000 });
    assert.ok(!result.error && result.signal === null,
      `TypeScript did not exit normally: signal=${result.signal}, error code=${result.error?.code ?? 'none'}, error message=${result.error?.message ?? 'none'}\n` +
      `stdout:\n${result.stdout ?? ''}\nstderr:\n${result.stderr ?? ''}`);
    return result;
  };
  const positive = check();
  assert.equal(positive.status, 0, `${positive.stdout}\n${positive.stderr}`);
  const negative = check(`options.upstream = 'http://127.0.0.2';
createProvider({ upstream: 42 });
provider.forward({ ...request, path: '/unrecognized' });
provider.forward({ ...request, body: [Buffer.from('{}')] });
const invalid: Promise<string> = provider.ready();
const badLogger: LoggerOptions = { level: 'verbose' };
const badProvider: ServerProvider = { ready: () => false };
args.command = 'serve';
authOptions.sessionCookie = 'changed';
auth.accessToken({ signal: 'invalid' });
createFramerBackend({ projectId: 'project', auth });
createFramerBackend({ projectId: 'project', auth, castle: {} });
createAccountBackend({ accountsFile: 'accounts.json' });
const invalidReference: AccountSessionReference = { env: 'SESSION', file: 'session.txt' };`);
  assert.equal(negative.status, 2, `${negative.stdout}\n${negative.stderr}`);
  assert.equal((negative.stdout.match(/error TS/g) ?? []).length, 14, negative.stdout);
  assert.match(negative.stdout, /TS2540/);
  assert.match(negative.stdout, /TS2322/);
  assert.match(negative.stdout, /TS2741/);
});
