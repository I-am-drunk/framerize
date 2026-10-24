// Inert process fixture. Never imports an SDK or account state.
import fs from 'node:fs';
const input = JSON.parse(fs.readFileSync(3, 'utf8')); fs.closeSync(3);
if (input.target.project_id === 'hang') await new Promise(() => setInterval(() => {}, 1000));
const operation = { version: 1, sdk: 'framer-embedded-v2', selection: 'configured-api',
  status: 'passed', failure_phase: null, device_failure: null,
  configure_calls: 1, token_method_calls: 1, device_storage_seeded: true,
  device_storage_unchanged: true, window_closed: true, sources_unchanged: true,
  async_errors: 0, diagnostics: 0, late_settlements: 0, method_ms: 2, elapsed_ms: 5 };
const report = { version: 1, kind: 'framer-v2-handoff', status: 'passed',
  failure_phase: null, operation, token_handoff: true, mint_to_handoff_ms: 1 };
if (input.target.project_id === 'refused') {
  Object.assign(report, { status: 'failed', failure_phase: 'operation',
    token_handoff: false, mint_to_handoff_ms: null });
  Object.assign(operation, { status: 'failed', failure_phase: 'device-storage',
    device_failure: 'cookie_count', device_storage_unchanged: false });
}
const token = Buffer.from('fake-fixture-token');
const header = Buffer.alloc(12); header.write('FSDK\x01\0\0\0');
header.writeUInt32BE(token.length, 8);
fs.writeSync(4, Buffer.concat([header, token])); fs.closeSync(4);
fs.writeSync(5, JSON.stringify(report)); fs.closeSync(5);
process.exit(['failed', 'refused'].includes(input.target.project_id) ? 2 : 0);
