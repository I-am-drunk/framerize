// Direct genuine V2 API component. It does not replace the V3 worker.
import { validateInput, OpaqueToken } from './protocol.mjs';
import { deviceFailureKind } from './device.mjs';

export class LegacyV2Operation {
  #input; #runtime; #used = false;
  constructor(input, runtime, residentPage = false) {
    this.#input = validateInput(input, residentPage);
    if (this.#input.version !== 2) throw new Error('saved-device-required');
    this.#runtime = runtime;
  }
  async mint() {
    if (this.#used) throw new Error('mint-already-used');
    this.#used = true;
    const input = this.#input, runtime = this.#runtime, started = runtime.now();
    const metadata = { version: 1, sdk: 'framer-embedded-v2', selection: 'configured-api',
      status: 'failed', failure_phase: null, device_failure: null,
      configure_calls: 0, token_method_calls: 0,
      device_storage_seeded: false, device_storage_unchanged: false,
      window_closed: false, sources_unchanged: false, async_errors: 0,
      diagnostics: 0, late_settlements: 0, method_ms: 0, elapsed_ms: 0 };
    let phase = 'input-identity', dom, verifyDevice, settlement, stopObservers;
    let value, mintedAt, accepting = true;
    try {
      runtime.verify();
      phase = 'runtime';
      runtime.admit();
      stopObservers = runtime.observe(metadata);
      phase = 'window';
      dom = await runtime.createWindow(input.target.url, metadata);
      phase = 'device-storage';
      verifyDevice = input.device.seed(dom.window);
      metadata.device_storage_seeded = true;
      phase = 'configure';
      const instance = await runtime.configure(dom.window, metadata);
      if (!instance || typeof instance.createRequestToken !== 'function')
        throw new Error('instance');
      verifyDevice();
      phase = 'mint';
      const mintStarted = runtime.now();
      const remaining = Math.min(5000, input.budget_ms - (mintStarted - started));
      if (remaining <= 0) throw new Error('deadline');
      settlement = await runtime.settle(() => {
        metadata.token_method_calls++;
        const result = instance.createRequestToken();
        if (!result || typeof result.then !== 'function') throw new Error('thenable');
        return Promise.resolve(result).then(token => {
          if (accepting) { value = token; mintedAt = runtime.now(); }
          return token;
        });
      }, remaining);
      metadata.method_ms = runtime.now() - mintStarted;
      if (settlement.outcome !== 'fulfilled' || settlement.nonempty_string !== true
          || metadata.method_ms > remaining)
        throw new Error('mint');
    } catch (error) {
      metadata.failure_phase = phase;
      metadata.device_failure = deviceFailureKind(error);
    }
    finally {
      accepting = false;
      if (dom && runtime.closeWorkers) {
        try { await runtime.closeWorkers(dom); }
        catch { metadata.failure_phase ??= 'cleanup'; }
      }
      if (verifyDevice) {
        try { verifyDevice(); metadata.device_storage_unchanged = true; }
        catch (error) {
          metadata.failure_phase ??= 'device-storage';
          metadata.device_failure ??= deviceFailureKind(error);
        }
      }
      if (dom) {
        try { dom.window.close(); metadata.window_closed = true; }
        catch { metadata.failure_phase ??= 'cleanup'; }
      }
      try { await runtime.drain(); }
      catch { metadata.failure_phase ??= 'cleanup'; }
      metadata.late_settlements = settlement?.late_settlements ?? 0;
      try { runtime.verify(); metadata.sources_unchanged = true; }
      catch { metadata.failure_phase ??= 'input-identity'; }
      try { stopObservers?.(); }
      catch { metadata.failure_phase ??= 'cleanup'; }
    }
    metadata.elapsed_ms = runtime.now() - started;
    if (!metadata.failure_phase && (metadata.configure_calls !== 1
        || metadata.token_method_calls !== 1 || !metadata.device_storage_seeded
        || !metadata.device_storage_unchanged || !metadata.window_closed
        || !metadata.sources_unchanged || metadata.async_errors !== 0
        || metadata.late_settlements !== 0)) metadata.failure_phase = 'cleanup';
    if (!metadata.failure_phase && (!Number.isFinite(metadata.elapsed_ms)
        || metadata.elapsed_ms < 0 || metadata.elapsed_ms > input.budget_ms))
      metadata.failure_phase = 'deadline';
    if (!metadata.failure_phase && (typeof value !== 'string' || !value.length
        || Buffer.byteLength(value, 'utf8') > 16384
        || Buffer.from(value).toString('utf8') !== value
        || !Number.isFinite(mintedAt))) metadata.failure_phase = 'token-shape';
    let token;
    if (!metadata.failure_phase) { token = new OpaqueToken(value); metadata.status = 'passed'; }
    value = undefined;
    return { token, metadata, minted_at: token ? mintedAt : undefined };
  }
}
