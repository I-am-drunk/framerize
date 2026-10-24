// Inert factory only. No SDK import, evaluation, mint or private input.
// Run only with the owner's qualified native-canvas DOM pins.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import vm from 'node:vm';
import { DOM } from '../../scripts/offline_worker/pins.mjs';
import { createV2Window } from '../../scripts/offline_worker/v2_runtime.mjs';

const jsdom = createRequire(import.meta.url)(DOM + '/node_modules/jsdom/lib/api.js');
const makeWindow = () => createV2Window(jsdom,
  'https://framer.com/projects/realm-fixture', new jsdom.VirtualConsole());
const constructors = ['Image', 'HTMLCanvasElement', 'CanvasRenderingContext2D'];
function nodeConstructorsAbsent() {
  for (const name of constructors) {
    assert.equal(Object.hasOwn(globalThis, name), false);
    assert.equal(typeof globalThis[name], 'undefined');
  }
}

const factorySource = `export default function(sdk) {
  sdk.configure = function({ window }) { return Object.freeze({
    imageType: typeof Image, canvasType: typeof HTMLCanvasElement,
    contextType: typeof CanvasRenderingContext2D,
    suppliedImageType: typeof window.Image,
    suppliedCanvasType: typeof window.HTMLCanvasElement,
    imageMatches: typeof Image === 'function' && Image === window.Image,
    canvasMatches: typeof HTMLCanvasElement === 'function' && HTMLCanvasElement === window.HTMLCanvasElement,
    imageDocumentMatches: typeof Image === 'function' && new Image().ownerDocument === window.document,
    globalMatches: globalThis === window,
  }); };
}`;

function facts(factory, window, bindReceiver = false) {
  const sdk = {};
  if (bindReceiver) factory.call(window, sdk);
  else factory(sdk);
  return { ...(bindReceiver
    ? sdk.configure.call(window, { window }) : sdk.configure({ window })) };
}
async function windowFactory(dom) {
  const module = new vm.SourceTextModule(factorySource, {
    context: dom.getInternalVMContext(), identifier: 'inert-v2-factory',
  });
  await module.link(() => { throw new Error('inert-factory-import'); });
  await module.evaluate({ timeout: 1000 });
  return module.namespace.default;
}

function availability(window) {
  const canvas = window.document.createElement('canvas');
  canvas.width = canvas.height = 2;
  return {
    imageType: typeof window.Image, canvasType: typeof window.HTMLCanvasElement,
    contextType: typeof window.CanvasRenderingContext2D,
    context2d: canvas.getContext('2d') !== null, webgl: canvas.getContext('webgl') !== null,
    offscreenType: typeof window.OffscreenCanvas, workerType: typeof window.Worker,
    sharedWorkerType: typeof window.SharedWorker,
    animationFrameType: typeof window.requestAnimationFrame,
    hidden: window.document.hidden,
  };
}

const expectedAvailability = {
  imageType: 'function', canvasType: 'function', contextType: 'undefined',
  context2d: true, webgl: false, offscreenType: 'undefined', workerType: 'undefined',
  sharedWorkerType: 'undefined', animationFrameType: 'undefined', hidden: true,
};
const nodeFacts = {
  imageType: 'undefined', canvasType: 'undefined', contextType: 'undefined',
  suppliedImageType: 'function', suppliedCanvasType: 'function',
  imageMatches: false, canvasMatches: false, imageDocumentMatches: false, globalMatches: false,
};
const windowFacts = { ...nodeFacts, imageType: 'function', canvasType: 'function',
  imageMatches: true, canvasMatches: true, imageDocumentMatches: true, globalMatches: true };

test('v2_node_factory_window_argument_and_receiver_do_not_rebind_constructors', { timeout: 5000 }, async () => {
  nodeConstructorsAbsent();
  const dom = makeWindow();
  try {
    assert.deepEqual(availability(dom.window), expectedAvailability);
    const url = 'data:text/javascript;base64,' + Buffer.from(factorySource).toString('base64');
    const { default: factory } = await import(url);
    assert.deepEqual(facts(factory, dom.window), nodeFacts);
    assert.deepEqual(facts(factory, dom.window, true), nodeFacts);
    assert.deepEqual(availability(dom.window), expectedAvailability);
  } finally { dom.window.close(); nodeConstructorsAbsent(); }
});

test('v2_window_factory_uses_native_window_constructors_without_global_installation', { timeout: 5000 }, async () => {
  nodeConstructorsAbsent();
  const dom = makeWindow();
  try {
    assert.deepEqual(availability(dom.window), expectedAvailability);
    const factory = await windowFactory(dom);
    assert.deepEqual(facts(factory, dom.window), windowFacts);
    assert.deepEqual(facts(factory, dom.window, true), windowFacts);
    assert.deepEqual(availability(dom.window), expectedAvailability);
  } finally { dom.window.close(); nodeConstructorsAbsent(); }
});

test('v2_factory_realms_keep_constructor_and_document_ownership_per_window', { timeout: 5000 }, async () => {
  nodeConstructorsAbsent();
  const first = makeWindow();
  let second;
  try {
    second = makeWindow();
    const firstFactory = await windowFactory(first), secondFactory = await windowFactory(second);
    assert.deepEqual(facts(firstFactory, first.window), windowFacts);
    assert.deepEqual(facts(secondFactory, second.window), windowFacts);
    const foreignFacts = { ...windowFacts, imageMatches: false, canvasMatches: false,
      imageDocumentMatches: false, globalMatches: false };
    assert.deepEqual(facts(firstFactory, second.window), foreignFacts);
    assert.deepEqual(facts(secondFactory, first.window), foreignFacts);
    assert.deepEqual(facts(firstFactory, second.window, true), foreignFacts);
    assert.deepEqual(facts(secondFactory, first.window, true), foreignFacts);
    assert.deepEqual(availability(first.window), expectedAvailability);
    assert.deepEqual(availability(second.window), expectedAvailability);
  } finally {
    try { second?.window.close(); }
    finally { first.window.close(); nodeConstructorsAbsent(); }
  }
});
