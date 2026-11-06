import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeFramerRequest, encodeFramerRequest } from '../dist/framer-request.js';

function inspect(content) {
  const request = { model: 'synthetic', messages: [{ role: 'user', content }] };
  const original = JSON.stringify(request);
  const decoded = decodeFramerRequest('/v1/chat/completions', Buffer.from(original));
  let reads = 0;
  let fields = 0;
  for (const block of decoded.request.messages[0].content) {
    for (const name of ['text', 'input_text']) {
      if (!Object.hasOwn(block, name)) continue;
      const value = block[name];
      fields += 1;
      Object.defineProperty(block, name, { enumerable: true,
        get() { reads += 1; return value; } });
    }
  }
  const envelope = JSON.parse(encodeFramerRequest(decoded, 'synthetic-project',
    'synthetic-token').toString());
  assert.equal(JSON.stringify(envelope.request), original);
  return { prompt: envelope.userPrompt, metadataReads: reads - fields };
}

for (const mode of ['long-first', 'empty-parts']) {
  test(`prompt metadata bounds content traversal for ${mode}`, { timeout: 5000 }, context => {
    const content = Array.from({ length: 10_000 }, (unused, index) => index % 2
      ? { input_text: mode === 'empty-parts' ? '' : 'unused' }
      : { text: mode === 'empty-parts' ? '' : 'unused' });
    if (mode === 'long-first') content[0] = { text: '🦊'.repeat(301) };
    const result = inspect(content);
    assert.equal(result.prompt, mode === 'long-first' ? '🦊'.repeat(300) : ' '.repeat(300));
    context.diagnostic(JSON.stringify({ mode, blocks: content.length,
      metadataReads: result.metadataReads, requestPreserved: true, runtime: process.version }));
    assert.ok(result.metadataReads <= 1202,
      'Metadata must not read text fields beyond its bounded prefix');
  });
}

test('short content retains separators, field priority and Unicode', () => {
  const content = [{ text: '' }, { text: 'chosen🦊', input_text: 'ignored' },
    { input_text: '\ud800' }, {}, { text: 'line\n\0end' }];
  const result = inspect(content);
  assert.equal(result.prompt, ' chosen🦊 \ud800  line\n\0end');
  assert.equal(result.metadataReads, 8);
});
