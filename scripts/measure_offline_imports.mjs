import fs from 'node:fs';
import Module, { createRequire } from 'node:module';
import { performance } from 'node:perf_hooks';
import { DOM } from './offline_worker/pins.mjs';

const result = { accepted: false, sdk_executed: false,
  account_inputs: false, authenticated_delivery: false };
const originalLoad = Module._load;
const prefix = DOM + '/node_modules/';
const stack = [];
const packages = new Map();
const largest = [];
let calls = 0;
let measured = 0;

function verifyIsolation() {
  const status = fs.readFileSync('/proc/self/status', 'utf8');
  if (process.version !== 'v24.13.0' || !/^NoNewPrivs:\s+1$/m.test(status)
      || !/^Seccomp:\s+2$/m.test(status)
      || !process.env.FRAMERIZE_PARENT_NETNS
      || fs.readlinkSync('/proc/self/ns/net') === process.env.FRAMERIZE_PARENT_NETNS
      || fs.existsSync('/home/tjninja/.framerize')
      || fs.existsSync('/run/user/1000/bus')) throw new Error('isolation');
  const group = '/sys/fs/cgroup' + fs.readFileSync('/proc/self/cgroup', 'utf8')
    .trim().split('::')[1];
  const number = name => Number(fs.readFileSync(group + '/' + name, 'utf8').trim());
  if (number('memory.max') !== 128 * 1024 ** 2 || number('memory.swap.max') !== 0
      || number('pids.max') !== 32) throw new Error('caps');
}

function measuredLoad(...args) {
  if (++calls > 6000 || stack.length > 128) throw new Error('probe-limit');
  const filename = Module._resolveFilename(args[0], args[1], args[2]);
  if (!filename.startsWith(prefix) || Module._cache[filename])
    return Reflect.apply(originalLoad, this, args);
  if (++measured > 2000) throw new Error('probe-limit');
  const relative = filename.slice(prefix.length);
  if (!/^[A-Za-z0-9_@./-]+$/.test(relative)) throw new Error('probe-label');
  const parts = relative.split('/node_modules/').at(-1).split('/');
  const name = parts[0].startsWith('@') ? parts.slice(0, 2).join('/') : parts[0];
  const frame = { children: 0 };
  const before = process.memoryUsage();
  const started = performance.now();
  stack.push(frame);
  try {
    return Reflect.apply(originalLoad, this, args);
  } finally {
    const after = process.memoryUsage();
    const elapsed = performance.now() - started;
    stack.pop();
    const inclusive = after.heapUsed - before.heapUsed;
    const net = inclusive - frame.children;
    if (stack.length) stack.at(-1).children += inclusive;
    recordLoad(name, relative, inclusive, net, elapsed, after.rss - before.rss);
  }
}

function recordLoad(name, filename, inclusive, net, elapsed, rssDelta) {
  let entry = packages.get(name);
  if (!entry) {
    if (packages.size >= 128) throw new Error('probe-limit');
    entry = { name, modules: 0, net_heap_delta_bytes: 0, negative_net_samples: 0 };
    packages.set(name, entry);
  }
  entry.modules++;
  entry.net_heap_delta_bytes += net;
  if (net < 0) entry.negative_net_samples++;
  largest.push({ filename, inclusive_heap_delta_bytes: inclusive,
    net_excluding_children_bytes: net, elapsed_ms: elapsed,
    process_rss_delta_bytes: rssDelta });
  largest.sort((a, b) => b.net_excluding_children_bytes - a.net_excluding_children_bytes);
  if (largest.length > 32) largest.pop();
}

try {
  verifyIsolation();
  result.before = process.memoryUsage();
  const started = performance.now();
  Module._load = measuredLoad;
  const jsdom = createRequire(import.meta.url)(prefix + 'jsdom/lib/api.js');
  Module._load = originalLoad;
  if (typeof jsdom.JSDOM !== 'function') throw new Error('module');
  result.module_load_ms = performance.now() - started;
  result.after = process.memoryUsage();
  result.reported_rss_peak_bytes = process.resourceUsage().maxRSS * 1024;
  result.accepted = measured > 0 && packages.size > 0;
} catch {
  result.failed = true;
} finally {
  Module._load = originalLoad;
}
result.hook_restored = Module._load === originalLoad;
result.module_load_calls = calls;
result.measured_uncached_modules = measured;
result.packages = [...packages.values()]
  .sort((a, b) => b.net_heap_delta_bytes - a.net_heap_delta_bytes);
result.largest_observed_module_deltas = largest;
result.limits = [
  'Instrumented import-only run; no SDK, token, account or HTTP request.',
  'Net heap deltas include GC and instrumentation effects.',
  'Nested module-load deltas cannot be added as separate memory ownership.',
  'Process RSS and group accounting are different measurements.',
];
fs.writeFileSync('/out/result.json', JSON.stringify(result, null, 2) + '\n',
  { flag: 'wx', mode: 0o600 });
process.exitCode = result.accepted && result.hook_restored ? 0 : 1;
