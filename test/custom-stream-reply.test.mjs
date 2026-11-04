import assert from 'node:assert/strict';
import test from 'node:test';
import { customCases } from '../.tools/scripts/fixtures/custom-route-scenario.js';
import { readCustomStream } from '../.tools/scripts/fixtures/custom-stream-reply.js';
import { responsesStream } from '../framerize-server/dist/compat/responses-stream.js';
import { messagesStream } from '../framerize-server/dist/compat/messages-stream.js';

for (const entry of customCases) {
  const render = entry.name === 'responses' ? responsesStream : messagesStream;
  for (const phase of ['first', 'final']) {
    test(`installed stream checker retains ${entry.name} ${phase} reply`, () => {
      const expected = JSON.parse(entry[phase]);
      const bytes = Buffer.concat([...render(expected)]);
      assert.deepEqual(readCustomStream(entry.name, bytes), expected);
    });
  }
  for (const [label, original, replacement] of [
    ['tool name', 'lookup', 'wrong_tool'],
    ['tool input', 'snow', 'rain'],
    ['tool identity', 'call_fixture', 'wrong_call'],
  ]) {
    test(`installed stream checker rejects corrupted ${entry.name} ${label}`, () => {
      const expected = JSON.parse(entry.first);
      const bytes = Buffer.from(Buffer.concat([...render(expected)]).toString().replaceAll(original, replacement));
      const terminal = entry.name === 'responses' ? 'response.completed' : 'message_stop';
      assert.ok(bytes.includes(`event: ${terminal}\n`));
      assert.ok(bytes.includes(entry.name === 'responses' ? 'response.function_call_arguments.delta' : 'input_json_delta'));
      assert.throws(() => assert.deepEqual(readCustomStream(entry.name, bytes), expected));
    });
  }
}
