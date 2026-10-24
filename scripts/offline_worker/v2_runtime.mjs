import fs from 'node:fs';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import { performance } from 'node:perf_hooks';
import { DOM, DOM_PINS, HISTORY, PUBLIC_KEY, SUPPORT } from './pins.mjs';
import { LegacyV2Operation } from './v2_operation.mjs';
import { pngResources } from './resources.mjs';

const FACTORY = HISTORY + '/runtime/framer-v2-factory.mjs';
const FACTORY_HASH = 'df5a150be914e7d267126f35de92934993e7930f18c3fd00bf79a86bd788ec58';
const count = (report, name) => { report[name] = Math.min(1000000, report[name] + 1); };
export function hashV2Input(path) {
  const digest = createHash('sha256'), buffer = Buffer.allocUnsafe(64 * 1024);
  const descriptor = fs.openSync(path, 'r');
  try {
    for (;;) {
      const count = fs.readSync(descriptor, buffer, 0, buffer.length, null);
      if (count === 0) break;
      digest.update(buffer.subarray(0, count));
    }
  } finally { fs.closeSync(descriptor); }
  return digest.digest('hex');
}
export function verifyV2Inputs() {
  const files = [[FACTORY, FACTORY_HASH], SUPPORT.settle,
    ...Object.entries(DOM_PINS).map(([name, hash]) => [DOM + '/' + name, hash])];
  for (const [path, expected] of files) {
    if (hashV2Input(path) !== expected)
      throw new Error('input-identity');
  }
}

export function createV2Window({ JSDOM, ResourceLoader }, url, virtualConsole) {
  const V2Resources = pngResources(ResourceLoader);
  return new JSDOM('<!doctype html><html><head></head><body></body></html>', {
    url, runScripts: 'outside-only', virtualConsole,
    resources: new V2Resources({}),
  });
}

const runtime = Object.freeze({
  now: () => performance.now(), verify: verifyV2Inputs,
  admit() {
    const status = fs.readFileSync('/proc/self/status', 'utf8');
    if (process.version !== 'v24.13.0' || !/^NoNewPrivs:\s+1$/m.test(status)
        || !/^Seccomp:\s+2$/m.test(status)
        || !process.execArgv.includes('--max-old-space-size=96'))
      throw new Error('runtime');
  },
  observe(report) {
    const failed = () => count(report, 'async_errors');
    const warning = () => count(report, 'diagnostics');
    process.on('unhandledRejection', failed);
    process.on('uncaughtExceptionMonitor', failed);
    process.on('warning', warning);
    return () => {
      process.removeListener('unhandledRejection', failed);
      process.removeListener('uncaughtExceptionMonitor', failed);
      process.removeListener('warning', warning);
    };
  },
  async createWindow(url, report) {
    const require = createRequire(import.meta.url);
    const { JSDOM, ResourceLoader, VirtualConsole } = require(DOM + '/node_modules/jsdom/lib/api.js');
    const console = new VirtualConsole();
    for (const name of ['warn', 'error', 'jsdomError'])
      console.on(name, () => count(report, 'diagnostics'));
    const dom = createV2Window({ JSDOM, ResourceLoader }, url, console);
    dom.window.addEventListener('error', () => count(report, 'async_errors'));
    return dom;
  },
  async configure(window, report) {
    const { default: factory } = await import(pathToFileURL(FACTORY));
    const sdk = {};
    factory(sdk);
    report.configure_calls++;
    return sdk.configure({ pk: PUBLIC_KEY, window });
  },
  async settle(invoke, budget) {
    const { settleOnce } = await import(pathToFileURL(SUPPORT.settle[0]));
    return settleOnce(invoke, budget);
  },
  async drain() {
    await new Promise(resolve => setImmediate(resolve));
    await new Promise(resolve => setImmediate(resolve));
  },
});
export function createLegacyV2Minter(input) { return new LegacyV2Operation(input, runtime); }
export function createResidentV2Minter(input) { return new LegacyV2Operation(input, runtime, true); }

export function createDedicatedV2Minter(input, residentPage = false) {
  let owner;
  const dedicated = Object.freeze({ ...runtime,
    async createWindow(url, report) {
      const dom = await runtime.createWindow(url, report);
      try {
        const { installV2DedicatedWorkers } = await import('./v2_dedicated_workers.mjs');
        owner = installV2DedicatedWorkers(dom.window,
          '11bba9bb64764c19f44154854b4ec9f66756a6ef95de25ef1a804db38c30b9bd');
        return dom;
      } catch (error) { dom.window.close(); throw error; }
    },
    async closeWorkers() {
      await owner.close();
      const state = owner.stats();
      if (state.started !== 1 || state.joined !== 1 || state.errors !== 0
          || state.messages !== 2) throw new Error('dedicated-worker-completion');
    },
  });
  return new LegacyV2Operation(input, dedicated, residentPage);
}
