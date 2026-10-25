// Offline fixture only. Synthetic values exercise the real native encoder.
import fs from 'node:fs';
import net from 'node:net';
import { spawnSync } from 'node:child_process';
import { readInput, verifyDescriptors } from './protocol.mjs';
verifyDescriptors();
const mode = readInput().target.project_id;
if (mode === 'hang') await new Promise(() => { setInterval(() => {}, 1000); });
const metadata = { version: 1, status: 'passed', failure_phase: null,
  target_validated: true, account_state_loaded: false, wrapper_calls: 1,
  branch: { v3: 1, v2: 0, none: 0, errors: 0 }, dynamic_imports: 1,
  v3_evaluated: true, window_closed: true, inspector_closed: true,
  sources_unchanged: true, async_errors: 0, diagnostics: 0,
  late_settlements: 0, method_ms: 2, elapsed_ms: 5,
  collection_to_handoff_ms: 1, collection_handoff: true };
if (mode === 'sandbox') {
  const child = spawnSync('/bin/true', [], { stdio: 'ignore' });
  if (child.error?.code !== 'EPERM') process.exit(4);
  const denied = await new Promise(resolve => {
    const server = net.createServer();
    server.on('error', error => resolve(error.code === 'EPERM'));
    server.listen(0, '127.0.0.1', () => server.close(() => resolve(false)));
  });
  if (!denied) process.exit(4);
}
if (mode === 'badmeta') metadata.branch.v2 = 1;
const token = Buffer.from(JSON.stringify({version: 2, shared_worker_reply: {kind: 'undefined'},
  values: {kind: 'array', values: Array.from({length: 765}, (_, i) =>
    ({kind: 'number', value: String(i)}))}}));
const header = Buffer.from([0x46, 0x43, 0x4c, 0x54, 1, 0, 0, 0, 0, 0, 0, 0]);
header.writeUInt32BE(token.length, 8);
let frame = Buffer.concat([header, token]);
if (mode === 'truncated') frame = frame.subarray(0, frame.length - 1);
if (mode === 'extra') frame = Buffer.concat([frame, Buffer.from([0])]);
if (mode === 'oversized') frame = Buffer.alloc(1024 * 1024 + 13, 1);
for (const [fd, bytes] of [[4, frame], [5, Buffer.from(JSON.stringify(metadata))]]) {
  let offset = 0;
  while (offset < bytes.length) {
    const count = fs.writeSync(fd, bytes, offset, bytes.length - offset);
    if (count <= 0) process.exit(5);
    offset += count;
  }
  if (mode === 'delayedclose' && fd === 4)
    await new Promise(resolve => setTimeout(resolve, 250));
  fs.closeSync(fd);
}
if (mode === 'delayedexit') await new Promise(resolve => setTimeout(resolve, 250));
if (mode === 'linger') await new Promise(() => { setInterval(() => {}, 1000); });
process.exit(mode === 'fail' ? 2 : 0);
