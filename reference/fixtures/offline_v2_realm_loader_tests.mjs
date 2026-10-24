import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { DOM } from '../../scripts/offline_worker/pins.mjs';
import { createV2Window } from '../../scripts/offline_worker/v2_runtime.mjs';
import { loadV2RealmFactory } from '../../scripts/offline_worker/v2_realm_factory.mjs';

const jsdom = createRequire(import.meta.url)(DOM + '/node_modules/jsdom/lib/api.js');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const code = text => Buffer.from(text);
function windowFor(t) {
  const dom = createV2Window(jsdom, 'https://framer.com/projects/fixture', new jsdom.VirtualConsole());
  t.after(() => dom.window.close());
  return dom;
}
const source = code(`export default sdk => {
  sdk.configure = ({window: supplied}) => ({
    sameWindow: supplied === window, sameImage: supplied.Image === Image,
    sameObject: Object.getPrototypeOf(sdk) === Object.prototype,
    image: () => new Image(), read: () => document.title,
    write: value => { document.title = value; }
  });
};`);

test('realm_factory_uses_actual_window_globals_and_realm_objects', async t => {
  const dom = windowFor(t), before = Object.getOwnPropertyDescriptor(globalThis, 'Image');
  const sdk = await loadV2RealmFactory(dom, source, hash(source));
  const api = sdk.configure({ window: dom.window });
  assert.equal(api.sameWindow, true); assert.equal(api.sameImage, true);
  assert.equal(api.sameObject, true);
  assert.ok(api.image() instanceof dom.window.HTMLImageElement);
  assert.deepEqual(Object.getOwnPropertyDescriptor(globalThis, 'Image'), before);
});

test('realm_factory_does_not_reuse_another_windows_module_or_document', async t => {
  const first = windowFor(t), second = windowFor(t);
  const a = (await loadV2RealmFactory(first, source, hash(source))).configure({ window: first.window });
  const b = (await loadV2RealmFactory(second, source, hash(source))).configure({ window: second.window });
  a.write('first'); b.write('second');
  assert.equal(a.read(), 'first'); assert.equal(b.read(), 'second');
  assert.equal(a.image().ownerDocument, first.window.document);
  assert.equal(b.image().ownerDocument, second.window.document);
});

test('realm_factory_refuses_unverified_bytes_before_entering_window', async () => {
  const dom = { getInternalVMContext() { assert.fail('window accessed before verification'); } };
  for (const bytes of [Buffer.alloc(0), Buffer.alloc(1024 * 1024 + 1), 'source'])
    await assert.rejects(loadV2RealmFactory(dom, bytes, hash(source)), /^Error: factory-identity$/);
  await assert.rejects(loadV2RealmFactory(dom, source, '0'.repeat(64)), /^Error: factory-identity$/);
  await assert.rejects(loadV2RealmFactory(dom, source, 'invalid'), /^Error: factory-identity$/);
});

test('realm_factory_refuses_invalid_utf8_without_entering_window', async () => {
  const bytes = Buffer.from([0xff]);
  const dom = { getInternalVMContext() { assert.fail('invalid encoding entered window'); } };
  await assert.rejects(loadV2RealmFactory(dom, bytes, hash(bytes)), /^Error: factory-encoding$/);
});

test('realm_factory_refuses_static_imports_before_module_side_effects', async t => {
  const dom = windowFor(t), bytes = code('import "node:fs"; document.title="executed"; export default () => {};');
  await assert.rejects(loadV2RealmFactory(dom, bytes, hash(bytes)), /^Error: factory-import$/);
  assert.equal(dom.window.document.title, '');
});

test('realm_factory_refuses_dynamic_imports_from_configured_api', async t => {
  const dom = windowFor(t);
  const bytes = code('export default sdk => { sdk.configure = () => import("node:fs"); };');
  const sdk = await loadV2RealmFactory(dom, bytes, hash(bytes));
  await assert.rejects(sdk.configure(), /^Error: factory-import$/);
});

test('realm_factory_requires_callable_export_and_configure', async t => {
  const dom = windowFor(t);
  for (const text of ['export default 1;', 'export const value = 1;']) {
    const bytes = code(text);
    await assert.rejects(loadV2RealmFactory(dom, bytes, hash(bytes)), /^Error: factory-export$/);
  }
  const bytes = code('export default sdk => { sdk.configure = 1; };');
  await assert.rejects(loadV2RealmFactory(dom, bytes, hash(bytes)), /^Error: factory-configure$/);
});

test('realm_factory_links_fresh_module_for_each_owned_window', async t => {
  const dom = windowFor(t);
  const bytes = code('let calls=0; export default sdk => { sdk.configure = () => ++calls; };');
  const first = await loadV2RealmFactory(dom, bytes, hash(bytes));
  const second = await loadV2RealmFactory(dom, bytes, hash(bytes));
  assert.equal(first.configure(), 1); assert.equal(first.configure(), 2);
  assert.equal(second.configure(), 1);
});
