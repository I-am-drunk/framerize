// Inert protocol fixture. Artificial token only; no SDK, account data or network.
import fs from 'node:fs';

const input = JSON.parse(fs.readFileSync(3, 'utf8'));
fs.closeSync(3);
if (input.version !== 2 || input.device.id !== 'device-bridge-fixture'
    || input.target.project_id !== 'success') throw new Error('fixture input');
const metadata = {
  version: 2, status: 'passed', failure_phase: null, target_validated: true,
  account_state_loaded: false, wrapper_calls: 1,
  branch: { v3: 1, v2: 0, none: 0, errors: 0 }, dynamic_imports: 1,
  v3_evaluated: true, window_closed: true, inspector_closed: true,
  sources_unchanged: true, async_errors: 0, diagnostics: 0,
  late_settlements: 0, method_ms: 2, elapsed_ms: 5,
  mint_to_handoff_ms: 1, token_handoff: true,
  device_storage_seeded: true, device_storage_unchanged: true,
};
const token = Buffer.from('fake-fixture-token');
const header = Buffer.from([0x46, 0x53, 0x44, 0x4b, 1, 0, 0, 0, 0, 0, 0, 0]);
header.writeUInt32BE(token.length, 8);
for (const [fd, bytes] of [[4, Buffer.concat([header, token])],
    [5, Buffer.from(JSON.stringify(metadata))]]) {
  let offset = 0;
  while (offset < bytes.length) {
    const count = fs.writeSync(fd, bytes, offset, bytes.length - offset);
    if (count <= 0) throw new Error('fixture write');
    offset += count;
  }
  fs.closeSync(fd);
}
