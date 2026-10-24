// Private descriptor handoff after a completed direct V2 operation.
const phases = [null, 'input-identity', 'runtime', 'window', 'device-storage',
  'configure', 'mint', 'cleanup', 'deadline', 'token-shape'];
const devices = [null, 'local_read', 'local_value', 'cookie_read',
  'cookie_count', 'cookie_value'];
function facts(m) {
  if (m?.version !== 1 || m.sdk !== 'framer-embedded-v2'
      || m.selection !== 'configured-api' || !['passed', 'failed'].includes(m.status)
      || !phases.includes(m.failure_phase) || !devices.includes(m.device_failure))
    throw new Error('operation-facts');
  const value = { version: 1, sdk: m.sdk, selection: m.selection,
    status: m.status, failure_phase: m.failure_phase, device_failure: m.device_failure };
  for (const key of ['device_storage_seeded', 'device_storage_unchanged',
    'window_closed', 'sources_unchanged']) {
    if (typeof m[key] !== 'boolean') throw new Error('operation-facts');
    value[key] = m[key];
  }
  for (const key of ['configure_calls', 'token_method_calls', 'async_errors',
    'diagnostics', 'late_settlements', 'method_ms', 'elapsed_ms']) {
    if (!Number.isFinite(m[key]) || m[key] < 0 || m[key] > 1000000)
      throw new Error('operation-facts');
    value[key] = m[key];
  }
  return value;
}
export async function runV2Handoff(ports) {
  let trusted = false, tokenClosed = false, phase = 'descriptors';
  const report = { version: 1, kind: 'framer-v2-handoff', status: 'failed',
    failure_phase: phase, operation: null, token_handoff: false,
    mint_to_handoff_ms: null };
  try {
    ports.verifyDescriptors(); trusted = true;
    phase = 'input';
    const minter = ports.createMinter(ports.readInput());
    phase = 'operation';
    const result = await minter.mint();
    report.operation = facts(result.metadata);
    if (report.operation.status !== 'passed' || !result.token)
      throw new Error('operation');
    phase = 'handoff';
    ports.verify();
    const write = await ports.writer();
    const age = () => {
      const value = ports.now() - result.minted_at;
      if (!Number.isFinite(value) || value < 0 || value > 2000)
        throw new Error('token-age');
      return value;
    };
    report.mint_to_handoff_ms = age();
    const token = result.token.take();
    try { write(token); } finally { tokenClosed = true; }
    report.mint_to_handoff_ms = age();
    report.token_handoff = true;
    report.status = 'passed'; report.failure_phase = null;
  } catch { report.failure_phase = phase; }
  finally { if (!tokenClosed) { try { ports.closeToken(); } catch {} } }
  if (trusted) {
    try { ports.writeMetadata(report); }
    catch { return 3; }
  }
  return report.status === 'passed' ? 0 : 2;
}
