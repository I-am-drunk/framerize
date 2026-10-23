const assert = require('node:assert/strict');
const { JSDOM, ResourceLoader, VirtualConsole } = require('./node_modules/jsdom');
const native = require('./node_modules/canvas');
assert.equal(process.version, 'v24.13.0');
assert.equal(native.version, '3.2.3');
let diagnostics = 0, blocked = 0, localImages = 0;
class Resources extends ResourceLoader {
  constructor() { super({ userAgent: 'Framerize/0.1' }); }
  fetch(url) {
    const match = /^data:image\/png;base64,([A-Za-z0-9+/]*={0,2})$/.exec(url);
    if (!match || match[1].length > 87384) { blocked++; return null; }
    const bytes = Buffer.from(match[1], 'base64');
    if (bytes.length > 65536 || bytes.toString('base64') !== match[1]) {
      blocked++; return null;
    }
    if (bytes.length < 33 || bytes.subarray(0, 8).toString('hex') !== '89504e470d0a1a0a'
        || bytes.readUInt32BE(8) !== 13 || bytes.toString('ascii', 12, 16) !== 'IHDR') {
      blocked++; return null;
    }
    const width = bytes.readUInt32BE(16), height = bytes.readUInt32BE(20);
    if (!width || !height || width > 2048 || height > 2048
        || width * height > 262144 || localImages >= 4) {
      blocked++; return null;
    }
    localImages++;
    const result = Promise.resolve(bytes);
    result.abort = () => {};
    return result;
  }
}
const console = new VirtualConsole();
console.on('jsdomError', () => diagnostics++);
const dom = new JSDOM('<!doctype html>', {
  url: 'https://fixture.invalid/', runScripts: 'outside-only',
  resources: new Resources(), virtualConsole: console,
});
async function main() {
  const canvas = dom.window.document.createElement('canvas');
  canvas.width = canvas.height = 2;
  assert.equal(canvas.getContext('webgl'), null);
  const context = canvas.getContext('2d');
  assert.ok(context);
  context.fillStyle = '#ff0000';
  context.fillRect(0, 0, 1, 1);
  assert.deepEqual(Array.from(context.getImageData(0, 0, 1, 1).data), [255, 0, 0, 255]);
  assert.deepEqual(Array.from(context.getImageData(1, 1, 1, 1).data), [0, 0, 0, 0]);
  const png = canvas.toDataURL('image/png');
  assert.ok(png.startsWith('data:image/png;base64,'));
  const image = new dom.window.Image();
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('image-timeout')), 2000);
    image.onload = () => { clearTimeout(timer); resolve(); };
    image.onerror = () => { clearTimeout(timer); reject(new Error('image-error')); };
    image.src = png;
  });
  const copy = dom.window.document.createElement('canvas');
  copy.width = copy.height = 2;
  copy.getContext('2d').drawImage(image, 0, 0);
  assert.deepEqual(Array.from(copy.getContext('2d').getImageData(0, 0, 1, 1).data),
    [255, 0, 0, 255]);
  assert.equal(canvas.getContext('webgl'), null);
  assert.equal(canvas.getContext('webgl2'), null);
  assert.equal(canvas.getContext('bitmaprenderer'), null);
  assert.equal(canvas.getContext('2d'), context);
  assert.equal(typeof dom.window.Worker, 'undefined');
  assert.equal(typeof dom.window.SharedWorker, 'undefined');
  const loader = new Resources();
  for (const url of ['https://fixture.invalid/a.png', 'file:///etc/passwd',
    'data:image/svg+xml,<svg/>', 'data:image/png;base64,AAAA']) {
    assert.equal(loader.fetch(url), null);
  }
  const huge = Buffer.from(png.split(',')[1], 'base64');
  huge.writeUInt32BE(65535, 16);
  assert.equal(loader.fetch('data:image/png;base64,' + huge.toString('base64')), null);
  assert.equal(diagnostics, 0);
  assert.equal(localImages, 1);
  assert.equal(blocked, 5);
  process.stdout.write(JSON.stringify({passed:true, synthetic:true, sdk_executed:false,
    native_canvas:native.version, cairo:native.cairoVersion, drawing:true,
    png_roundtrip:true, blocked_inputs:blocked, diagnostics,
    webgl_supported:false, workers_supported:false}) + '\n');
}
main().catch(error => { process.stderr.write(error.name + ': ' + error.message + '\n');
  process.exitCode = 1;
}).finally(() => dom.window.close());
