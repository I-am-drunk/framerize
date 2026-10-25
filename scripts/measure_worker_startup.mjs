// Measure dependency startup only; no SDK or account operation is invoked.
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { performance } from 'node:perf_hooks';

const [root, output] = process.argv.slice(2);
if (!root || !output || !path.isAbsolute(root) || !path.isAbsolute(output))
  throw new Error('expected absolute runtime root and new output file');
const started = performance.now();
const samples = [];
function snapshot(phase) {
  const text = fs.readFileSync('/proc/self/smaps_rollup', 'utf8');
  const kib = name => {
    const values = [...text.matchAll(new RegExp(`^${name}:\\s+(\\d+) kB$`, 'gm'))];
    if (values.length !== 1) throw new Error('incomplete memory observation');
    const value = Number(values[0][1]);
    if (!Number.isSafeInteger(value)) throw new Error('invalid memory observation');
    return value;
  };
  const memory = process.memoryUsage();
  samples.push({ phase, elapsed_ms: performance.now() - started,
    rss_kib: kib('Rss'), pss_kib: kib('Pss'),
    private_kib: kib('Private_Clean') + kib('Private_Dirty') + kib('Private_Hugetlb'),
    heap_used_bytes: memory.heapUsed, heap_total_bytes: memory.heapTotal,
    external_bytes: memory.external, array_buffers_bytes: memory.arrayBuffers });
}
snapshot('node_startup');
const require = createRequire(import.meta.url);
const { JSDOM, VirtualConsole } = require(path.join(root, 'inputs/dom/node_modules/jsdom/lib/api.js'));
snapshot('jsdom_loaded');
let dom;
try {
  dom = new JSDOM('<!doctype html><html><head></head><body></body></html>', {
    url: 'https://example.invalid/', runScripts: 'outside-only',
    pretendToBeVisual: true, virtualConsole: new VirtualConsole(),
  });
  snapshot('window_created');
} finally {
  dom?.window.close();
}
snapshot('window_closed');
fs.writeFileSync(output, JSON.stringify({
  runtime: process.version, scope: 'single Node process; snapshots, not peaks',
  operation: 'dependency import and blank window only',
  sdk_evaluated: false, forced_gc: false, samples,
}) + '\n', { flag: 'wx', mode: 0o600 });
