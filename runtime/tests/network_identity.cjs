const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync(process.argv[2], 'utf8');
const start = source.indexOf('function readRequestTurnId(');
const end = source.indexOf('function requestHeaders(', start);
assert.ok(start >= 0 && end > start);
let enabled = true, body, reads = 0;
const context = vm.createContext({
  Services: {prefs: {getBoolPref: () => enabled}},
  readRequestPostData: (_channel, identityOnly) => {
    assert.equal(identityOnly, true);
    reads++;
    return body;
  },
});
vm.runInContext(source.slice(start, end), context);
const read = method => context.readRequestTurnId({requestMethod: method});
body = 'realTurn9';
assert.equal(read('POST'), 'realTurn9');
enabled = false;
assert.equal(read('POST'), undefined);
enabled = true;
assert.equal(read('GET'), undefined);
assert.equal(reads, 1);
for (const turnId of [null, 4, {}, '', 'x'.repeat(65), 'a b', 'a\nb']) {
  body = turnId;
  assert.equal(read('POST'), undefined);
}
body = 'invalid identity!';
assert.equal(read('POST'), undefined);
console.log('Bounded request identity checks passed.');

// An optional diagnostic must never consume an upload it cannot clone.
class UploadChannel {}
class CloneableStream {}
class SeekableStream {}
class TellableStream {}
let consumed = 0;
const uploadContext = vm.createContext({
  Ci: {nsIUploadChannel: UploadChannel,
    nsICloneableInputStream: CloneableStream,
    nsISeekableStream: SeekableStream, nsITellableStream: TellableStream},
  NetUtil: {readInputStreamToString: () => {
    consumed++;
    return '{"turnId":"realTurn9"}';
  }},
});
const uploadStart = source.indexOf('function readRequestPostData(');
vm.runInContext(source.slice(uploadStart, start), uploadContext);
const upload = new UploadChannel();
upload.uploadStream = {available: () => 22};
assert.equal(uploadContext.readRequestPostData(upload, true), undefined);
assert.equal(consumed, 0, 'diagnostics consumed the original upload');
console.log('Uncloneable upload remains untouched.');
