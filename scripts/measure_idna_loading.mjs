// Synthetic equivalence checks for deferred Unicode mapping-table loading.
import fs from 'node:fs';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { performance } from 'node:perf_hooks';

const result = { accepted: false, synthetic: true, sdk_executed: false,
  authenticated_delivery: false, cases: 0 };
const started = performance.now();
const mode = process.argv[2];
const group = '/sys/fs/cgroup' + fs.readFileSync('/proc/self/cgroup', 'utf8')
  .trim().split('::')[1];
const number = name => Number(fs.readFileSync(group + '/' + name, 'utf8').trim());
function requireIsolation() {
  const status = fs.readFileSync('/proc/self/status', 'utf8');
  if (process.version !== 'v24.13.0' || process.platform !== 'linux'
      || process.arch !== 'x64' || !/^NoNewPrivs:\s+1$/m.test(status)
      || !/^Seccomp:\s+2$/m.test(status)
      || !process.execArgv.includes('--max-old-space-size=96')
      || !process.execArgv.includes('--experimental-vm-modules')
      || !process.execArgv.includes('--jitless')
      || !process.env.FRAMERIZE_PARENT_NETNS
      || fs.readlinkSync('/proc/self/ns/net') === process.env.FRAMERIZE_PARENT_NETNS
      || fs.existsSync('/home/tjninja/.framerize')
      || fs.existsSync('/run/user/1000/bus')) throw new Error('isolation');
  if (number('memory.max') !== 128 * 1024 ** 2 || number('memory.swap.max') !== 0
      || number('pids.max') !== 32) throw new Error('caps');
}

try {
  requireIsolation();
  if (!['plain', 'lazy'].includes(mode)) throw new Error('mode');
  const pins = JSON.parse(fs.readFileSync('/work/idna-inputs.json', 'utf8'));
  const digest = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
  const verify = () => {
    if (digest(pins.path) !== pins[mode] || digest(pins.table) !== pins.table_sha256)
      throw new Error('input-changed');
  };
  verify();
  const require = createRequire(import.meta.url);
  const tr46 = require(pins.path);
  const table = require.resolve(pins.table);
  result.table_loaded_at_import = Boolean(require.cache[table]);
  if (result.table_loaded_at_import !== (mode === 'plain')) throw new Error('import-state');
  const flags = ['checkHyphens', 'checkBidi', 'checkJoiners', 'useSTD3ASCIIRules',
    'verifyDNSLength', 'transitionalProcessing', 'ignoreInvalidPunycode'];
  const options = Array.from({ length: 128 }, (_, mask) =>
    Object.fromEntries(flags.map((flag, index) => [flag, Boolean(mask & (1 << index))])));
  const hash = createHash('sha256');
  function check(domain, settings = options) {
    for (const option of settings) for (const method of ['toASCII', 'toUnicode']) {
      let value;
      try { value = { result: tr46[method](domain, option) }; }
      catch (error) { value = { error: error.name, message: error.message }; }
      hash.update(JSON.stringify([domain, option, method, value]) + '\n');
      result.cases++;
    }
  }
  for (let point = 0; point < 128; point++) {
    const c = String.fromCodePoint(point);
    check(c + 'a.example');
    check('a' + c + 'b.example');
    check('a.example' + c);
    if (performance.now() - started > 18000) throw new Error('deadline');
  }
  for (const domain of ['', '.', 'framer.com', 'EXAMPLE.COM', 'a..b', '-a.b',
    'a-.b', 'ab--cd.example', 'a'.repeat(63) + '.com', 'a'.repeat(64) + '.com']) check(domain);
  result.ascii_cases = result.cases;
  result.table_loaded_after_ascii = Boolean(require.cache[table]);
  if (mode === 'lazy' && result.table_loaded_after_ascii) throw new Error('eager-table');
  for (const domain of ['bücher.example', 'faß.de', 'ＦＲＡＭＥＲ.com', '例え.テスト',
    'a\u200Db.example', '\u0301a.example', 'אa.example', 'مثال.إختبار',
    'ςσ.example', '😀.example', '\uD800.example', 'xn--bcher-kva.example',
    'xn--invalid-.example', 'xn--.example', 'a\u3002b', '\u0000ä.example']) check(domain);
  result.table_loaded_after_unicode = Boolean(require.cache[table]);
  if (!result.table_loaded_after_unicode) throw new Error('missing-unicode-table');
  check('FRAMER.com');
  result.output_sha256 = hash.digest('hex');
  if (result.cases !== 105216) throw new Error('case-count');
  verify();
  result.inputs_unchanged = true;
  result.accepted = true;
} catch (error) {
  result.failure_phase = error.message;
}
result.mode = mode;
result.elapsed_ms = performance.now() - started;
result.reported_rss_peak_bytes = process.resourceUsage().maxRSS * 1024;
fs.writeFileSync('/out/result.json', JSON.stringify(result, null, 2) + '\n',
  { flag: 'wx', mode: 0o600 });
process.exitCode = result.accepted ? 0 : 2;
