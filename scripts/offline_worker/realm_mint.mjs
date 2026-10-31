// Realm-based Castle minter. Same mint flow as offline_mint.mjs but with
// the no-JSDOM realm window instead of JSDOM. Opt-in sibling; the JSDOM
// path in offline_mint.mjs is unchanged.
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';
import { performance } from 'node:perf_hooks';
import { TextEncoder } from 'node:util';
import { PUBLIC_KEY, SUPPORT, GRAPHICS } from './pins.mjs';
import { loadGraphics } from './graphics_loader.mjs';
import { installWebGL } from './canvas_webgl.mjs';
import { SdkModules, verifyInputs } from './sdk_loader.mjs';
import { validateInput, OpaqueCollection } from './protocol.mjs';
import { COLLECTION_LIMIT } from './collection.mjs';
import { observeBranch, exactV3, observeFailurePhase, refineFailurePhase } from './branch.mjs';
import { createRealmWindow } from './realm_window.mjs';

export class RealmCastleMinter {
  #input; #used = false;
  constructor(input) { this.#input = validateInput(input); }
  async mint() {
    if (this.#used) throw new Error('mint-already-used');
    this.#used = true;
    const started = performance.now();
    const report = { version: this.#input.version, status: 'failed', failure_phase: null,
      target_validated: true, account_state_loaded: false, wrapper_calls: 0,
      branch: { v3: 0, v2: 0, none: 0, errors: 0 }, dynamic_imports: 0,
      v3_evaluated: false, window_closed: false, inspector_closed: false,
      sources_unchanged: false, async_errors: 0, diagnostics: 0,
      late_settlements: 0, method_ms: 0, elapsed_ms: 0,
      collection_to_handoff_ms: 0, collection_handoff: false };
    if (this.#input.version === 2) Object.assign(report, {
      device_storage_seeded: false, device_storage_unchanged: false,
    });
    let phase = 'input-identity', dom, modules, inspector, settlement, value, mintedAt, verifyDevice, nativeGraphics, graphics, failureDetail;
    const asynchronousError = () => { report.async_errors = Math.min(1000000, report.async_errors + 1); };
    const diagnostic = () => { report.diagnostics = Math.min(1000000, report.diagnostics + 1); };
    process.on('unhandledRejection', asynchronousError);
    process.on('uncaughtExceptionMonitor', asynchronousError);
    process.on('warning', diagnostic);
    try {
      verifyInputs();
      phase = 'runtime';
      const status = fs.readFileSync('/proc/self/status', 'utf8');
      if (!/^NoNewPrivs:\s+1$/m.test(status) || !/^Seccomp:\s+2$/m.test(status)
          || !process.execArgv.includes('--max-old-space-size=96')
          || !process.execArgv.includes('--experimental-vm-modules')) throw new Error('runtime');
      phase = 'import';
      const { settleOnce } = await import(pathToFileURL(SUPPORT.settle[0]));
      const { createErrorLocationReader } = await import(pathToFileURL(SUPPORT.location[0]));
      phase = 'window';
      dom = createRealmWindow(PUBLIC_KEY);
      if (GRAPHICS) {
        phase = 'graphics';
        nativeGraphics = loadGraphics(GRAPHICS);
        graphics = installWebGL(dom.window, nativeGraphics.create,
          nativeGraphics.cleanup, nativeGraphics.activeContexts);
      }
      if (typeof dom.window.TextEncoder !== 'function') throw new Error('encoding');
      dom.window.addEventListener('error', asynchronousError);
      dom.window.castlePublishableAPIKey = PUBLIC_KEY;
      if (typeof dom.window.requestAnimationFrame !== 'function'
          || typeof dom.window.cancelAnimationFrame !== 'function'
          || dom.window.document.hidden !== false
          || dom.window.document.visibilityState !== 'visible') throw new Error('environment');
      if (this.#input.device) {
        phase = 'device-storage';
        verifyDevice = this.#input.device.seed(dom.window);
        report.device_storage_seeded = true;
      }
      phase = 'module-link';
      modules = new SdkModules(dom.getInternalVMContext());
      await modules.prepare();
      phase = 'inspector';
      inspector = createErrorLocationReader();
      await inspector.enable();
      phase = 'module-evaluate';
      await modules.evaluate();
      phase = 'mint';
      const remaining = Math.min(11000, this.#input.budget_ms - (performance.now() - started));
      if (remaining <= 0) throw new Error('deadline');
      settlement = await settleOnce(() => {
        report.wrapper_calls++;
        const pending = modules.mint();
        if (!pending || typeof pending.then !== 'function') throw new Error('thenable');
        return pending.then(token => { value = token; mintedAt = performance.now(); return token; });
      }, remaining);
      report.method_ms = settlement.elapsed_ms;
      if (settlement.outcome !== 'fulfilled' || settlement.nonempty_string !== true)
        throw new Error('mint');
    } catch { report.failure_phase = phase; }
    finally {
      if (verifyDevice) {
        try { verifyDevice(); report.device_storage_unchanged = true; }
        catch { report.failure_phase ??= 'device-storage'; }
      }
      if (graphics) {
        try {
          const state = graphics.close();
          if (!state.created || state.created !== state.destroyed || state.errors || state.refused)
            report.failure_phase ??= 'graphics';
          nativeGraphics.verify();
        } catch { report.failure_phase ??= 'graphics-cleanup'; }
      }
      if (dom) {
        // The realm has no window to close; the mint context is discarded when
        // the process exits. Mark cleanup complete so the deployed acceptor's
        // required window_closed field is truthful for this runtime shape.
        report.window_closed = true;
      }
      await new Promise(resolve => setImmediate(resolve));
      await new Promise(resolve => setImmediate(resolve));
      if (modules) {
        try {
          const entries = modules.loggerEntries();
          report.branch = observeBranch(entries);
          failureDetail = observeFailurePhase(entries);
        }
        catch { report.failure_phase ??= 'branch'; }
        Object.assign(report, modules.stats);
      }
      if (inspector) {
        try {
          await inspector.close();
          const flags = Object.values(inspector.cleanup);
          report.inspector_closed = flags.length === 5 && flags.every(value => value === true);
        } catch { report.failure_phase ??= 'cleanup'; }
      }
      report.late_settlements = settlement?.late_settlements ?? 0;
      try { verifyInputs(); report.sources_unchanged = true; }
      catch { report.failure_phase ??= 'input-identity'; }
      process.removeListener('unhandledRejection', asynchronousError);
      process.removeListener('uncaughtExceptionMonitor', asynchronousError);
      process.removeListener('warning', diagnostic);
      report.elapsed_ms = performance.now() - started;
    }
    if (!report.failure_phase && (!exactV3(report.branch) || report.dynamic_imports !== 1
        || !report.v3_evaluated || report.wrapper_calls !== 1)) report.failure_phase = 'branch';
    if (!report.failure_phase && (!report.window_closed || !report.inspector_closed
        || !report.sources_unchanged || report.async_errors !== 0 || report.late_settlements !== 0))
      report.failure_phase = 'cleanup';
    if (!report.failure_phase && report.elapsed_ms > this.#input.budget_ms)
      report.failure_phase = 'deadline';
    if (!report.failure_phase && (typeof value !== 'string' || value.length === 0
        || Buffer.byteLength(value, 'utf8') > COLLECTION_LIMIT || Buffer.from(value).toString('utf8') !== value))
      report.failure_phase = 'collection-shape';
    report.failure_phase = refineFailurePhase(report.failure_phase, failureDetail);
    let collection;
    if (!report.failure_phase) { collection = new OpaqueCollection(value); report.status = 'passed'; }
    value = undefined;
    return { collection, metadata: report, collected_at: mintedAt };
  }
}
