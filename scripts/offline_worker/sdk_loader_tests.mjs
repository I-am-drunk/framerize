import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';

const CDN = 'https://app.framerstatic.com/';
const modern = `export function a() { globalThis.initialized++; }
  export async function b() { globalThis.generated++;
    return (await import('${CDN}v3.mjs')).value; }`;
async function fixture(t, name = 'b', wrapper = modern, options = {}) {
  const root = await mkdtemp(join(tmpdir(), 'sdk-loader-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const sources = { wrapper, v3: 'export const value = "fixture-only";' };
  const modules = {};
  for (const [role, source] of Object.entries(sources)) {
    await writeFile(join(root, role + '.mjs'), source);
    modules[role] = [role + '.mjs', [], createHash('sha256').update(source).digest('hex')];
  }
  if (options.badHash) modules.wrapper[2] = '0'.repeat(64);
  const values = { ASSETS: root, DOM: root, CDN, MODULES: modules,
    SUPPORT: {}, DOM_PINS: {}, WRAPPER_MINT_EXPORT: name };
  await writeFile(join(root, 'pins.mjs'), Object.entries(values)
    .map(([key, value]) => `export const ${key} = ${JSON.stringify(value)};`).join('\n'));
  await writeFile(join(root, 'sdk_loader.mjs'),
    await readFile(new URL('./sdk_loader.mjs', import.meta.url)));
  const { SdkModules } = await import(pathToFileURL(join(root, 'sdk_loader.mjs')));
  const context = vm.createContext({ initialized: 0, generated: 0 });
  return { context, modules: new SdkModules(context) };
}

test('current_export_mints_without_calling_initializer', async t => {
  const { modules, context } = await fixture(t);
  await modules.prepare(); await modules.evaluate();
  assert.equal(await modules.mint(), 'fixture-only');
  assert.equal(context.initialized, 0); assert.equal(context.generated, 1);
  assert.deepEqual(modules.stats, { dynamic_imports: 1, v3_evaluated: true });
});
test('legacy_export_remains_supported_by_its_pin', async t => {
  const { modules } = await fixture(t, 'a', 'export async function a(){return "legacy"}');
  await modules.prepare(); await modules.evaluate();
  assert.equal(await modules.mint(), 'legacy');
});
test('missing_pinned_export_never_falls_back_to_initializer', async t => {
  const { modules, context } = await fixture(t, 'missing');
  await modules.prepare(); await modules.evaluate();
  assert.throws(() => modules.mint(), { message: 'wrapper-unavailable' });
  assert.equal(context.initialized, 0); assert.equal(context.generated, 0);
});
test('unevaluated_wrapper_cannot_mint', async t => {
  const { modules } = await fixture(t);
  await modules.prepare();
  assert.throws(() => modules.mint(), { message: 'wrapper-unavailable' });
});
test('changed_asset_is_rejected_before_execution', async t => {
  await assert.rejects(fixture(t, 'b', modern, { badHash: true }), { message: 'input-identity' });
});
test('undeclared_static_dependency_is_rejected', async t => {
  const { modules } = await fixture(t, 'b', `import '${CDN}v3.mjs'; ${modern}`);
  await assert.rejects(modules.prepare(), { message: 'dependency-set' });
});
test('undeclared_dynamic_dependency_is_rejected', async t => {
  const { modules } = await fixture(t, 'b',
    `export async function b(){return import('${CDN}other.mjs')}`);
  await modules.prepare(); await modules.evaluate();
  await assert.rejects(modules.mint(), { message: 'unexpected-import' });
});
