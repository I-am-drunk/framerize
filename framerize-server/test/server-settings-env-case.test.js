import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { connect } from 'node:net';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const compiled = await build({ absWorkingDir: root, bundle: true, write: false,
  platform: 'node', format: 'esm', metafile: true, stdin: { resolveDir: root, contents:
    "export * from './framerize-server/src/settings.ts'; export { loadConfig } from './framerize-server/src/config.ts'; export { createServer } from './framerize-server/src/server.ts';" },
  plugins: [{ name: 'current-core', setup(builder) {
    builder.onResolve({ filter: /\/dist\/core\.js$/ }, () => ({ path: `${root}framerize-core/src/index.ts` }));
  } }],
});
const bundle = compiled.outputFiles[0].contents;
console.log(JSON.stringify({ bundleSha256: createHash('sha256').update(bundle).digest('hex'),
  inputs: Object.keys(compiled.metafile.inputs).sort() }));
const api = await import(`data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`);

function windowsLookup(values) {
  return new Proxy(values, { get(target, property) {
    if (typeof property !== 'string') return Reflect.get(target, property);
    const key = Object.keys(target).find(name => name.toLowerCase() === property.toLowerCase());
    return target[key ?? property];
  } });
}

test('saved environment names retain fallback and explicit undefined precedence', async context => {
  const directory = await mkdtemp(join(tmpdir(), 'server-env-fallback-'));
  context.after(() => rm(directory, { recursive: true, force: true }));
  const path = join(directory, 'server.json');
  for (const name of ['constructor', '__proto__']) {
    const saved = JSON.stringify({ version: 1, config: { serverTokenEnv: name },
      environment: { [name]: 'synthetic_saved_token' } });
    await writeFile(path, saved, { mode: 0o600 });
    const loaded = await api.configuredSettings(path, {}, {});
    assert.equal(loaded.config.serverToken, 'synthetic_saved_token');
    await assert.rejects(api.configuredSettings(path, {}, { [name]: undefined }), /server token/);
    assert.equal(await readFile(path, 'utf8'), saved);
  }
});

function get(port, token) {
  return new Promise((resolve, reject) => {
    const request = http.request({ host: '127.0.0.1', port, path: '/ready', agent: false,
      headers: token ? { authorization: `Bearer ${token}` } : {} }, response => {
      response.on('error', reject);
      response.resume();
      response.once('end', () => resolve(response.statusCode));
    });
    request.on('error', reject);
    request.setTimeout(1500, () => request.destroy(new Error('Fixture request timed out')));
    request.end();
  });
}

function refused(port) {
  return new Promise((resolve, reject) => {
    const socket = connect(port, '127.0.0.1');
    socket.setTimeout(1500, () => socket.destroy(new Error('Fixture port check timed out')));
    socket.once('connect', () => { socket.destroy(); reject(new Error('Fixture port remains open')); });
    socket.once('error', error => {
      socket.destroy();
      if (error.code === 'ECONNREFUSED') resolve(); else reject(error);
    });
  });
}

for (const scenario of [
  { name: 'mixed-case token without saved settings', saved: false, mixed: true },
  { name: 'mixed-case token replaces the saved token', saved: true, mixed: true },
  { name: 'uppercase token replaces the saved token', saved: true, mixed: false },
]) {
  test(scenario.name, { timeout: 6000 }, async context => {
    const rootPath = await mkdtemp(join(tmpdir(), 'server-env-case-'));
    const path = join(rootPath, 'server.json');
    let instance;
    let port;
    let closes = 0;
    context.after(async () => {
      try {
        await instance?.close();
        if (port) await refused(port);
      } finally { await rm(rootPath, { recursive: true, force: true }); }
    });
    const token = 'new_synthetic_server_token';
    const stale = 'old_synthetic_server_token';
    const values = { backend: 'http', host: '127.0.0.1', port: 0 };
    const raw = { [scenario.mixed ? 'FrAmErIzE_SeRvEr_ToKeN' : 'FRAMERIZE_SERVER_TOKEN']: token };
    const env = scenario.mixed ? windowsLookup(raw) : raw;
    const initialEnv = JSON.stringify(raw);
    let saved;
    if (scenario.saved) {
      const previous = { FRAMERIZE_SERVER_TOKEN: stale };
      saved = `${JSON.stringify(api.captureSettings(api.loadConfig(values, previous), previous), null, 2)}\n`;
      await writeFile(path, saved, { mode: 0o600 });
    }
    assert.equal(api.loadConfig(values, env).serverToken, token);
    const loaded = await api.configuredSettings(scenario.saved ? path : undefined, values, env);
    let forwards = 0;
    instance = await api.createServer(loaded.config, { provider: {
      ready: async () => true,
      forward: async () => { forwards += 1; return { status: 200, headers: {}, body: [] }; },
      close: async () => { closes += 1; },
    } });
    port = (await instance.listen()).port;
    const observed = {
      selectedCurrentToken: loaded.config.serverToken === token,
      selectedStaleToken: loaded.config.serverToken === stale,
      unauthenticated: await get(port), currentToken: await get(port, token),
      staleToken: await get(port, stale),
    };
    await instance.close();
    await refused(port);
    assert.equal(closes, 1);
    assert.equal(forwards, 0);
    assert.equal(JSON.stringify(raw), initialEnv);
    if (saved) assert.equal(await readFile(path, 'utf8'), saved);
    else await assert.rejects(readFile(path), { code: 'ENOENT' });
    context.diagnostic(JSON.stringify(observed));
    assert.deepEqual(observed, { selectedCurrentToken: true, selectedStaleToken: false,
      unauthenticated: 401, currentToken: 200, staleToken: 401 });
  });
}
