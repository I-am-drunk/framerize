// Synthetic local SDK measurement. No credential inputs or token output.
import fs from 'node:fs';
import v8 from 'node:v8';
import { performance } from 'node:perf_hooks';

const output = '/out/result.json';
const result = { schema: 1, accepted: false, synthetic: true,
  authenticated_delivery: false, samples: [] };
const started = performance.now();
let phase = 'isolation';
const group = '/sys/fs/cgroup' + fs.readFileSync('/proc/self/cgroup', 'utf8')
  .trim().split('::')[1];
const number = name => Number(fs.readFileSync(group + '/' + name, 'utf8').trim());
function sample(name) {
  const memory = process.memoryUsage();
  const spaces = v8.getHeapSpaceStatistics().map(space => ({
    name: space.space_name, size: space.space_size, used: space.space_used_size,
    physical: space.physical_space_size,
  }));
  result.samples.push({ phase: name, elapsed_ms: performance.now() - started,
    memory, spaces, max_rss_bytes: process.resourceUsage().maxRSS * 1024 });
}
function requireIsolation() {
  const status = fs.readFileSync('/proc/self/status', 'utf8');
  if (process.version !== 'v24.13.0' || process.platform !== 'linux'
      || process.arch !== 'x64' || !/^NoNewPrivs:\s+1$/m.test(status)
      || !/^Seccomp:\s+2$/m.test(status)
      || !process.execArgv.includes('--max-old-space-size=96')
      || !process.execArgv.includes('--experimental-vm-modules')
      || !process.env.FRAMERIZE_PARENT_NETNS
      || fs.readlinkSync('/proc/self/ns/net') === process.env.FRAMERIZE_PARENT_NETNS
      || fs.existsSync('/home/tjninja/.framerize')
      || fs.existsSync('/run/user/1000/bus')) throw new Error('isolation');
  if (number('memory.max') !== 128 * 1024 ** 2 || number('memory.swap.max') !== 0
      || number('pids.max') !== 32) throw new Error('caps');
}
try {
  requireIsolation();
  const permitted = ['--max-old-space-size=96', '--experimental-vm-modules',
    '--max-semi-space-size=1', '--optimize-for-size', '--jitless'];
  if (process.execArgv.some(flag => !permitted.includes(flag))) throw new Error('flags');
  result.flags = process.execArgv;
  sample('node_entry');
  phase = 'load';
  const { OfflineCastleMinter } = await import('./offline_worker/offline_mint.mjs');
  sample('worker_loaded');
  phase = 'mint';
  const minter = new OfflineCastleMinter({ version: 2, budget_ms: 16000,
    target: { project_id: 'fixture-project',
      url: 'https://framer.com/projects/fixture-project', path: '/ai/v3/responses/' },
    device: { id: 'synthetic-memory-fixture' } });
  const minted = await minter.mint();
  let token = minted.token?.take();
  result.token_discarded = typeof token === 'string' && token.length > 0;
  token = undefined;
  const m = minted.metadata;
  result.sdk = Object.fromEntries(['status', 'failure_phase', 'wrapper_calls',
    'dynamic_imports', 'v3_evaluated', 'window_closed', 'inspector_closed',
    'sources_unchanged', 'async_errors', 'late_settlements', 'method_ms',
    'elapsed_ms', 'device_storage_seeded', 'device_storage_unchanged']
    .map(key => [key, m[key]]));
  if (!result.token_discarded || m.status !== 'passed' || m.wrapper_calls !== 1
      || m.dynamic_imports !== 1 || !m.v3_evaluated || !m.window_closed
      || !m.inspector_closed || !m.sources_unchanged || m.async_errors !== 0
      || m.late_settlements !== 0 || !m.device_storage_seeded
      || !m.device_storage_unchanged) throw new Error('mint');
  sample('mint_returned_token_discarded');
  result.accepted = true;
} catch { result.failure_phase = phase; }
try {
  result.cgroup = group;
  result.resources = Object.fromEntries(['memory.current', 'memory.peak',
    'memory.swap.current', 'memory.swap.peak', 'memory.max', 'memory.swap.max',
    'pids.current', 'pids.peak', 'pids.max'].map(name => [name, number(name)]));
  result.memory_events = Object.fromEntries(fs.readFileSync(group + '/memory.events', 'utf8')
    .trim().split('\n').map(line => { const [name, value] = line.split(' '); return [name, Number(value)]; }));
  if (result.resources['memory.swap.current'] || result.resources['memory.swap.peak']
      || Object.values(result.memory_events).some(value => value !== 0)) result.accepted = false;
  result.active_resources = process.getActiveResourcesInfo();
  result.thread_count = fs.readdirSync('/proc/self/task').length;
} catch { result.accepted = false; result.resource_capture_failed = true; }
result.elapsed_ms = performance.now() - started;
fs.writeFileSync(output, JSON.stringify(result, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
process.exitCode = result.accepted ? 0 : 1;
