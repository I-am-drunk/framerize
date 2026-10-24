import fs from 'node:fs';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import { performance } from 'node:perf_hooks';
import { TextEncoder } from 'node:util';
import { DOM, PUBLIC_KEY, SUPPORT } from './pins.mjs';
import { SdkModules, verifyInputs } from './sdk_loader.mjs';
import { validateInput, OpaqueToken } from './protocol.mjs';
import { observeBranch, exactV3 } from './branch.mjs';
import { pngResources } from './resources.mjs';
import { installSharedWorkers } from './shared_workers.mjs';
import { installDedicatedWorkers } from './dedicated_workers.mjs';

export function createOfflineWindow({ JSDOM, ResourceLoader }, url, virtualConsole) {
  const OfflineResources = pngResources(ResourceLoader);
  // Preserve native identity. Only bounded, inline PNG images may load.
  return new JSDOM('<!doctype html><html><head></head><body></body></html>', {
    url, runScripts: 'outside-only', pretendToBeVisual: true, virtualConsole,
    resources: new OfflineResources(),
  });
}

export class OfflineCastleMinter {
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
      mint_to_handoff_ms: 0, token_handoff: false };
    if (this.#input.version === 2) Object.assign(report, {
      device_storage_seeded: false, device_storage_unchanged: false,
    });
    let phase = 'input-identity', dom, modules, inspector, settlement, value, mintedAt, verifyDevice, sharedWorkers, dedicatedWorkers;
    const asynchronousError = () => { report.async_errors = Math.min(1000000, report.async_errors + 1); };
    const diagnostic = () => { report.diagnostics = Math.min(1000000, report.diagnostics + 1); };
    process.on('unhandledRejection', asynchronousError);
    process.on('uncaughtExceptionMonitor', asynchronousError);
    process.on('warning', diagnostic);
    try {
      verifyInputs();
      phase = 'runtime';
      const status = fs.readFileSync('/proc/self/status', 'utf8');
      if (process.version !== 'v24.13.0' || !/^NoNewPrivs:\s+1$/m.test(status)
          || !/^Seccomp:\s+2$/m.test(status)
          || !process.execArgv.includes('--max-old-space-size=96')
          || !process.execArgv.includes('--experimental-vm-modules')) throw new Error('runtime');
      phase = 'import';
      const require = createRequire(import.meta.url);
      const jsdom = require(DOM + '/node_modules/jsdom/lib/api.js');
      const { settleOnce } = await import(pathToFileURL(SUPPORT.settle[0]));
      const { createErrorLocationReader } = await import(pathToFileURL(SUPPORT.location[0]));
      phase = 'window';
      const console = new jsdom.VirtualConsole();
      for (const name of ['warn', 'error', 'jsdomError']) console.on(name, diagnostic);
      dom = createOfflineWindow(jsdom, this.#input.target.url, console);
      if (this.#input.device) {
        phase = 'device-storage';
        verifyDevice = this.#input.device.seed(dom.window);
        report.device_storage_seeded = true;
      }
      if (typeof dom.window.TextEncoder !== 'undefined') throw new Error('encoding');
      Object.defineProperty(dom.window, 'TextEncoder', {
        value: TextEncoder, configurable: true, writable: true,
      });
      if (dom.window.TextEncoder !== TextEncoder) throw new Error('encoding');
      dom.window.addEventListener('error', asynchronousError);
      dom.window.castlePublishableAPIKey = PUBLIC_KEY;
      sharedWorkers = installSharedWorkers(dom.window,
        'cb1716096c87eff9d6f2296bbac1302c0b3c58e404226b15cd45cd223c4addab');
      dedicatedWorkers = installDedicatedWorkers(dom.window, [
        'f9bbca7372aaea93762997eb78a3a3c3198132b22bcb787754b64059087ad4ff',
        '7fbadc72ebc1e56a4274d005ec9ce05ad2435bbcc2406f8640712bbdd0987176',
      ]);
      if (typeof dom.window.requestAnimationFrame !== 'function'
          || typeof dom.window.cancelAnimationFrame !== 'function'
          || dom.window.document.hidden !== false
          || dom.window.document.visibilityState !== 'visible'
          || dom.window.Worker !== dedicatedWorkers.Worker
          || dom.window.SharedWorker !== sharedWorkers.SharedWorker
          || typeof dom.window.OffscreenCanvas !== 'undefined'
          || typeof dom.window.indexedDB !== 'undefined') throw new Error('environment');
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
        return pending.then(token => {
          value = token; mintedAt = performance.now(); return token;
        });
      }, remaining);
      report.method_ms = settlement.elapsed_ms;
      if (settlement.outcome !== 'fulfilled' || settlement.nonempty_string !== true)
        throw new Error('mint');
    } catch { report.failure_phase = phase; }
    finally {
      if (dedicatedWorkers) {
        try {
          const state = await dedicatedWorkers.close();
          const replies = dedicatedWorkers.replyStats();
          if (state.errors || state.started !== 2 || state.joined !== 2
              || replies.feature !== 1 || replies.entropy !== 1 || replies.timing !== 1
              || replies.error !== 0 || replies.other !== 0)
            report.failure_phase ??= 'dedicated-worker';
        } catch { report.failure_phase ??= 'cleanup'; }
      }
      if (sharedWorkers) {
        try {
          const state = await sharedWorkers.close();
          if (state.started !== 1 || state.joined !== 1 || state.errors !== 0
              || state.messages !== 1 || state.sdk_replies !== 1)
            report.failure_phase ??= 'shared-worker';
        } catch { report.failure_phase ??= 'cleanup'; }
      }
      if (verifyDevice) {
        try { verifyDevice(); report.device_storage_unchanged = true; }
        catch { report.failure_phase ??= 'device-storage'; }
      }
      if (dom) {
        try { dom.window.close(); report.window_closed = true; }
        catch { report.failure_phase ??= 'cleanup'; }
      }
      await new Promise(resolve => setImmediate(resolve));
      await new Promise(resolve => setImmediate(resolve));
      if (modules) {
        try { report.branch = observeBranch(modules.loggerEntries()); }
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
    }
    report.elapsed_ms = performance.now() - started;
    if (!report.failure_phase && (!exactV3(report.branch) || report.dynamic_imports !== 1
        || !report.v3_evaluated || report.wrapper_calls !== 1)) report.failure_phase = 'branch';
    if (!report.failure_phase && (!report.window_closed || !report.inspector_closed
        || !report.sources_unchanged || report.async_errors !== 0 || report.late_settlements !== 0))
      report.failure_phase = 'cleanup';
    if (!report.failure_phase && report.elapsed_ms > this.#input.budget_ms)
      report.failure_phase = 'deadline';
    if (!report.failure_phase && (typeof value !== 'string' || value.length === 0
        || Buffer.byteLength(value, 'utf8') > 16384 || Buffer.from(value).toString('utf8') !== value))
      report.failure_phase = 'token-shape';
    let token;
    if (!report.failure_phase) { token = new OpaqueToken(value); report.status = 'passed'; }
    value = undefined;
    return { token, metadata: report, minted_at: mintedAt };
  }
}
