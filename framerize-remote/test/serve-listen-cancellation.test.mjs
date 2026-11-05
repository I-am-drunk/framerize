import assert from 'node:assert/strict';
import dns from 'node:dns';
import http from 'node:http';
import { once } from 'node:events';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import test from 'node:test';

let run;
if (process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT) {
  ({ run } = await import(pathToFileURL(join(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT, 'dist/index.js')).href));
} else {
  const { build } = await import('esbuild');
  const result = await build({ entryPoints: [fileURLToPath(new URL('../src/cli.ts', import.meta.url))],
    bundle: true, platform: 'node', format: 'esm', write: false,
    alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) } });
  ({ run } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].contents).toString('base64')}`));
}

async function settlesWithin(promise) {
  let timer;
  try { return await Promise.race([promise.then(() => true),
    new Promise(resolve => { timer = setTimeout(() => resolve(false), 1000); })]); }
  finally { clearTimeout(timer); }
}

for (const event of ['SIGINT', 'SIGTERM']) {
  test(`${event} settles serve while listener DNS remains pending`, { timeout: 5000 }, async t => {
    const root = await mkdtemp(join(tmpdir(), 'frm-listen-cancel-'));
    const oldEnv = process.env;
    const before = ['SIGINT', 'SIGTERM'].map(name => process.listeners(name));
    process.env = { HOME: root, USERPROFILE: root, FRM_HOME: root, PATH: oldEnv.PATH };
    t.after(async () => { process.env = oldEnv; await rm(root, { recursive: true, force: true }); });
    let requests = 0;
    const upstream = http.createServer((request, response) => {
      assert.equal(request.url, '/ready');
      requests += 1;
      response.setHeader('content-type', 'application/json');
      response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
    });
    upstream.listen(0, '127.0.0.1');
    await once(upstream, 'listening');
    t.after(async () => {
      upstream.closeAllConnections();
      await new Promise(resolve => upstream.close(resolve));
    });
    const profile = join(root, 'profile.json');
    await writeFile(profile, JSON.stringify({ remote: `http://127.0.0.1:${upstream.address().port}`,
      host: 'pending-listen.invalid', port: 0, allowNonLoopback: true,
      localToken: { kind: 'value', value: 'synthetic-local-token' } }));
    let enter;
    const entered = new Promise(resolve => { enter = resolve; });
    const original = dns.lookup;
    let releaseDns = () => {};
    t.mock.method(dns, 'lookup', (host, options, callback) => {
      if (host !== 'pending-listen.invalid') return original(host, options, callback);
      let released = false;
      releaseDns = () => {
        if (released) return;
        released = true;
        callback(null, [{ address: '127.0.0.1', family: 4 }]);
      };
      enter();
    });
    const output = [], errors = [];
    const operation = run(['serve', '--config', profile, '--json'], {
      stdout: text => output.push(text), stderr: text => errors.push(text),
    }).then(code => ({ code }), error => ({ error }));
    t.after(async () => { process.emit(event); releaseDns(); await operation; });
    await Promise.race([entered, operation.then(result => {
      throw new Error(`serve ended before listener DNS: ${JSON.stringify(result)}`);
    })]);
    process.emit(event);
    const stoppedBeforeDns = await settlesWithin(operation);
    const outputBeforeDns = output.join('');
    releaseDns();
    const result = await operation;
    assert.equal(stoppedBeforeDns, true, 'serve must finish before DNS is released');
    assert.equal(outputBeforeDns, '{"stopped":true}\n');
    assert.deepEqual(result, { code: 0 });
    assert.equal(output.join(''), '{"stopped":true}\n');
    assert.deepEqual(errors, []);
    assert.equal(requests, 1);
    assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => process.listeners(name)), before);
  });
}
