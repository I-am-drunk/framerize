import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeFramerRequest, encodeFramerRequest } from '../dist/framer-request.js';

test('bounded metadata preserves the original Unicode prefix at UTF-16 boundaries', () => {
  for (const padding of [0, 1, 298, 299, 300, 301]) {
    const text = `${'x'.repeat(padding)}${'🦊'.repeat(310)}`;
    const expected = [...text].slice(0, 300).join('');
    for (const content of [false, true]) {
      const request = content ? { model: 'synthetic', messages: [
        { role: 'user', content: [{ type: 'text', text }] },
      ] } : { model: 'synthetic', input: text };
      const path = content ? '/v1/chat/completions' : '/v1/responses';
      const bytes = Buffer.from(JSON.stringify(request));
      const envelope = JSON.parse(encodeFramerRequest(decodeFramerRequest(path, bytes),
        'synthetic-project', 'synthetic-token').toString());
      assert.equal(envelope.userPrompt, expected);
      assert.equal(JSON.stringify(envelope.request), bytes.toString());
    }
  }
});
