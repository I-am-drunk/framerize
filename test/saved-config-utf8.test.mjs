import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build } from 'esbuild';

const sourceRoot = process.env.FRAMERIZE_SOURCE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const digest = value => createHash('sha256').update(value).digest('hex');
async function compile(contents) {
  const compiled = await build({ absWorkingDir: sourceRoot, stdin: { contents,
    resolveDir: sourceRoot }, bundle: true, platform: 'node', format: 'esm',
  write: false, metafile: true });
  const inputs = await Promise.all(Object.keys(compiled.metafile.inputs)
    .filter(path => path !== '<stdin>').sort().map(async path => ({
      path, sha256: digest(await readFile(resolve(sourceRoot, path))),
    })));
  const bundle = compiled.outputFiles[0].contents;
  console.log(JSON.stringify({ node: process.version, platform: process.platform,
    inputs, bundleSha256: digest(bundle) }));
  return import(`data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`);
}

const server = await compile(`
  export { configuredSettings } from './framerize-server/src/settings.ts';
  export { readServerToken } from './framerize-server/src/config.ts';`);
const remote = await compile(`
  export { loadConfigFile } from './framerize-remote/src/configStore.ts';
  export { loadCredential } from './framerize-remote/src/credentials.ts';`);
const readers = [{
  name: 'Server settings',
  document: path => ({ version: 1, config: { backend: 'http',
    host: '127.0.0.1', port: 0, serverTokenFile: path }, environment: {} }),
  load: async path => (await server.configuredSettings(path, {}, {})).config,
  credential: config => server.readServerToken(config),
  path: config => config.serverTokenFile,
  invalid: error => error.message === 'Server settings are unavailable or invalid',
}, {
  name: 'Remote profile',
  document: path => ({ remote: 'http://127.0.0.1:8789', port: 0,
    serverToken: { kind: 'file', path } }),
  load: path => remote.loadConfigFile(path),
  credential: config => remote.loadCredential(config.serverToken, {}),
  path: config => config.serverToken.path,
  invalid: error => error.code === 'profile_invalid',
}];
const malformed = [
  ['invalid leading byte', Buffer.from([0xff])],
  ['incomplete sequence', Buffer.from([0xe2, 0x82])],
  ['overlong encoding', Buffer.from([0xc0, 0xaf])],
];

async function directory(context) {
  const root = await mkdtemp(join(tmpdir(), 'saved-config-utf8-'));
  context.after(async () => {
    await rm(root, { recursive: true, force: true });
    await assert.rejects(readFile(join(root, 'config.json')), { code: 'ENOENT' });
    context.diagnostic('private state removed');
  });
  return root;
}

for (const reader of readers) {
  test(`${reader.name} rejects malformed UTF-8 before selecting credentials`, { timeout: 3000 }, async context => {
    const root = await directory(context);
    const configPath = join(root, 'config.json');
    const observations = [];
    for (const [label, invalid] of malformed) {
      const text = JSON.stringify(reader.document(join(root, 'token-ENCODING_MARKER')));
      const [prefix, suffix] = text.split('ENCODING_MARKER');
      const bytes = Buffer.concat([Buffer.from(prefix), invalid, Buffer.from(suffix)]);
      assert.throws(() => new TextDecoder('utf-8', { fatal: true }).decode(bytes));
      const replacementPath = join(root, `token-${invalid.toString('utf8')}`);
      const token = 'synthetic-other-file';
      await writeFile(replacementPath, token, { mode: 0o600 });
      await writeFile(configPath, bytes, { mode: 0o600 });
      let config, error;
      try { config = await reader.load(configPath); } catch (failure) { error = failure; }
      const observation = { label, rejectedAsInvalid: Boolean(error && reader.invalid(error)) };
      if (config) {
        observation.selectedReplacementPath = reader.path(config) === replacementPath;
        observation.loadedOtherCredentialFile = await reader.credential(config) === token;
      }
      observations.push(observation);
      assert.deepEqual(await readFile(configPath), bytes);
      assert.equal(await readFile(replacementPath, 'utf8'), token);
    }
    context.diagnostic(JSON.stringify(observations));
    assert.equal(observations.every(result => result.rejectedAsInvalid), true,
      'Malformed UTF-8 must not silently select replacement-character credential paths');
  });
}

for (const reader of readers) {
  test(`${reader.name} preserves valid Unicode credential paths`, { timeout: 3000 }, async context => {
    const root = await directory(context);
    const tokenPath = join(root, 'token-caf\u00e9-\u{1f680}-\ufffd');
    const configPath = join(root, 'config.json');
    const token = 'synthetic-utf8-control';
    const bytes = Buffer.from(JSON.stringify(reader.document(tokenPath)));
    await writeFile(tokenPath, token, { mode: 0o600 });
    await writeFile(configPath, bytes, { mode: 0o600 });
    const config = await reader.load(configPath);
    assert.equal(reader.path(config), tokenPath);
    assert.equal(await reader.credential(config), token);
    assert.deepEqual(await readFile(configPath), bytes);
    assert.equal(await readFile(tokenPath, 'utf8'), token);
  });
}
