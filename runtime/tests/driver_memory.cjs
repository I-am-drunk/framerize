'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const source = require('node:fs').readFileSync(process.argv[2], 'utf8');
const start = source.indexOf('class PipeFrameDecoder {');
const end = source.indexOf('// packages/utils/pipeTransport.ts', start);
assert.ok(start > 0 && end > start);
const PipeFrameDecoder = require('node:vm').runInNewContext(
  source.slice(start, end) + '; PipeFrameDecoder', {Buffer});

function frame(body, endian = 'le') {
  const data = Buffer.isBuffer(body) ? body : Buffer.from(body);
  const head = Buffer.alloc(4);
  head[endian === 'le' ? 'writeUInt32LE' : 'writeUInt32BE'](data.length);
  return Buffer.concat([head, data]);
}

for (const endian of ['le', 'be']) {
  test(endian + ': arbitrary fragmentation preserves messages and UTF-8', () => {
    const expected = ['plain', 'snowman \u2603, \ud83c\udf0d, \u00e9'.repeat(120),
      'tail', 'x'.repeat(18000)];
    const bytes = Buffer.concat(expected.map(value => frame(value, endian)));
    for (const seed of [1, 19, 42, 8123]) {
      const actual = [];
      const parser = new PipeFrameDecoder(endian, value => actual.push(value));
      let random = seed, offset = 0;
      while (offset < bytes.length) {
        random = (Math.imul(random, 1664525) + 1013904223) >>> 0;
        const count = 1 + random % 257;
        parser.push(bytes.subarray(offset, offset + count));
        offset += count;
      }
      assert.deepEqual(actual, expected);
    }
  });
}

test('invalid and incomplete UTF-8 follows Buffer decoding', () => {
  const body = Buffer.from([0x61, 0xe2, 0x82, 0xac, 0xff, 0xc3, 0x28, 0xf0, 0x9f]);
  const bytes = frame(body);
  for (let split = 1; split < bytes.length; split++) {
    const actual = [];
    const parser = new PipeFrameDecoder('le', value => actual.push(value));
    parser.push(bytes.subarray(0, split));
    parser.push(bytes.subarray(split));
    assert.deepEqual(actual, [body.toString('utf8')]);
  }
});

test('partial input is owned and cannot be changed by its sender', () => {
  const actual = [];
  const parser = new PipeFrameDecoder('le', value => actual.push(value));
  const first = frame('original').subarray(0, 9);
  parser.push(first);
  first.fill(0);
  parser.push(Buffer.from('nal'));
  assert.deepEqual(actual, ['original']);
});

test('huge declared frame does not allocate its declared length', () => {
  const parser = new PipeFrameDecoder('le', () => assert.fail('incomplete'));
  parser.push(Buffer.from([0xff, 0xff, 0xff, 0xff]));
  assert.equal(parser.frame, null);
  parser.push(Buffer.from('x'));
  assert.ok(parser.frame.length <= 8192);
});

test('independent sessions retain no completed body buffers', () => {
  const left = [], right = [];
  const a = new PipeFrameDecoder('le', message => left.push(message));
  const b = new PipeFrameDecoder('le', message => right.push(message));
  const large = 'a'.repeat(2 * 1024 * 1024);
  const bytes = frame(large);
  for (let i = 0; i < bytes.length; i += 65536) {
    a.push(bytes.subarray(i, i + 65536));
    b.push(frame('other'));
  }
  assert.deepEqual(left, [large]);
  assert.ok(right.length > 1 && right.every(value => value === 'other'));
  assert.equal(a.frame, null);
  assert.equal(b.frame, null);
});

test('empty transport frames do not prevent following valid messages', () => {
  const actual = [];
  const parser = new PipeFrameDecoder('le', value => actual.push(value));
  parser.push(Buffer.concat([frame(''), frame('one'), frame(''), frame('two')]));
  assert.deepEqual(actual, ['one', 'two']);
});

test('fragmented large frames require only linear copying', () => {
  const data = frame('a'.repeat(1024 * 1024));
  const copy = Buffer.prototype.copy;
  let copied = 0;
  Buffer.prototype.copy = function(...args) {
    const count = copy.apply(this, args); copied += count; return count;
  };
  try {
    const actual = [];
    const decoder = new PipeFrameDecoder('le', value => actual.push(value));
    for (let i = 0; i < data.length; i += 17) decoder.push(data.subarray(i, i + 17));
    assert.deepEqual(actual, ['a'.repeat(1024 * 1024)]);
    assert.ok(copied < data.length * 3, `copied ${copied} bytes for ${data.length}`);
  } finally { Buffer.prototype.copy = copy; }
});

test('console eviction releases only handles never exposed to a caller', () => {
  const begin = source.indexOf('      addConsoleMessage(worker,');
  const end = source.indexOf('      consoleMessages(filter)', begin);
  const limitBegin = source.indexOf('function ensureArrayLimit(');
  const limitEnd = source.indexOf('var PageEvent', limitBegin);
  assert.ok(begin > 0 && end > begin && limitEnd > limitBegin);
  class ConsoleMessage {
    constructor(_page, _worker, _type, _text, args) { this.values = args; }
    args() { return this.values; }
  }
  const Page = require('node:vm').runInNewContext(
    source.slice(limitBegin, limitEnd) + 'class Page {' + source.slice(begin, end) + '}; Page',
    {ConsoleMessage, process: {env: {FRAMERIZE_DISPOSE_EVICTED_CONSOLE: '1'}}});
  const page = new Page();
  page._consoleMessages = [];
  page.frameManager = {interceptConsoleMessage: () => false};
  const handles = Array.from({length: 1000}, (_, i) => ({
    _framerizeConsoleHandleExposed: i % 37 === 0, disposed: false,
    dispose() { this.disposed = true; }
  }));
  for (const handle of handles) page.addConsoleMessage(null, 'log', [handle]);
  assert.ok(page._consoleMessages.length <= 200);
  assert.ok(handles[1].disposed);
  assert.ok(!handles.at(-1).disposed);
  page.clearConsoleMessages();
  for (const handle of handles) assert.equal(handle.disposed, !handle._framerizeConsoleHandleExposed);
});
