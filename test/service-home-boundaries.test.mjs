import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

for (const [name, relative, key] of [
  ['Remote', '../framerize-remote/src/serviceUnits.ts', 'FRM_HOME'],
  ['Server', '../framerize-server/src/service-units.ts', 'FRAMERIZE_SERVER_HOME'],
]) {
  const result = await build({
    entryPoints: [fileURLToPath(new URL(relative, import.meta.url))],
    bundle: true, platform: 'node', format: 'esm', write: false,
  });
  const { serviceHomeOf } = await import(`data:text/javascript;base64,${
    Buffer.from(result.outputFiles[0].contents).toString('base64')}`);

  test(`${name} reads complete fields at physical line boundaries`, () => {
    for (const newline of ['\n', '\r\n', '\r']) {
      const field = `Environment=${key}=/synthetic/home`;
      assert.equal(serviceHomeOf(`${field}${newline}Restart=always`), '/synthetic/home');
      assert.equal(serviceHomeOf(`[Service]${newline}${field}`), '/synthetic/home');
    }
  });

  test(`${name} does not read a directive after a Unicode separator`, () => {
    for (const separator of ['\u2028', '\u2029']) {
      const decoy = `Description=example${separator}Environment=${key}=/decoy`;
      assert.equal(serviceHomeOf(decoy), undefined);
      assert.equal(serviceHomeOf(`${decoy}\nEnvironment=${key}=/actual`), '/actual');
    }
  });
}
