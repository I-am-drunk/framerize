import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/cli.ts', import.meta.url))],
  alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
});
stop();
const hash = value => createHash('sha256').update(value).digest('hex');
const inputs = Object.keys(compiled.metafile.inputs).sort();
assert.ok(inputs.every(path => !/castle|framerize-server/.test(path)));
console.log(JSON.stringify({ runtime: process.version, bundle: hash(compiled.outputFiles[0].contents),
  inputs: await Promise.all(inputs.map(async path => [path, hash(await readFile(path))])) }));
const { run } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

for (const command of ['serve', 'setup']) {
  test(`${command} help describes non-loopback flag as optional compatibility`, { timeout: 2000 }, async () => {
    let stdout = '', stderr = '';
    const code = await run([command, '--config', '/synthetic/missing-profile.json', '--help'], {
      stdout(text) { stdout += text; }, stderr(text) { stderr += text; },
    });
    assert.equal(code, 0);
    assert.equal(stderr, '');
    assert.match(stdout, /--host ADDRESS/);
    assert.match(stdout, /--port NUMBER/);
    const description = stdout.split('\n').find(line => line.trimStart().startsWith('--allow-non-loopback'));
    assert.ok(description, 'The compatibility flag remains documented');
    assert.match(description, /accepted for older profiles; no longer required/i);
  });
}
