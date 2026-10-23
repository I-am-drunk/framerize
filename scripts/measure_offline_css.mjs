import fs from 'node:fs';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { performance } from 'node:perf_hooks';
import { DOM } from './offline_worker/pins.mjs';

const result = { accepted: false, sdk_executed: false, account_inputs: false };
const hash = value => createHash('sha256').update(value).digest('hex');
const mode = process.argv[2];
const cssPath = DOM + '/node_modules/cssstyle/lib/CSSStyleDeclaration.js';
const pins = JSON.parse(fs.readFileSync('/work/css-inputs.json', 'utf8'));

function verifyIsolation() {
  const status = fs.readFileSync('/proc/self/status', 'utf8');
  if (!['plain', 'layout'].includes(mode) || process.version !== 'v24.13.0'
      || !/^NoNewPrivs:\s+1$/m.test(status) || !/^Seccomp:\s+2$/m.test(status)
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

function checkPrototype(Constructor) {
  const prototype = Constructor.prototype;
  const keys = Reflect.ownKeys(prototype);
  if (keys.length > 5000) throw new Error('fixture-bound');
  const descriptors = [];
  const behavior = createHash('sha256');
  let setters = 0;
  for (const key of keys) {
    const d = Object.getOwnPropertyDescriptor(prototype, key);
    const label = typeof key === 'symbol' ? 'symbol:' + key.description : key;
    descriptors.push([label, d.enumerable, d.configurable, d.writable ?? null,
      typeof d.value, typeof d.value === 'function' ? hash(d.value.toString()) : d.value,
      d.get ? hash(d.get.toString()) : null, d.set ? hash(d.set.toString()) : null]);
    if (typeof key !== 'string' || !d.set || ['cssText', 'length'].includes(key)) continue;
    setters++;
    const style = new Constructor();
    let observation;
    try {
      style[key] = 'initial';
      observation = [label, style[key], style.cssText, style.length];
    } catch (error) { observation = [label, 'error', error.name]; }
    const encoded = JSON.stringify(observation);
    if (encoded.length > 16384) throw new Error('fixture-bound');
    behavior.update(encoded + '\n');
  }
  return { property_count: keys.length, setters_checked: setters,
    descriptors_sha256: hash(JSON.stringify(descriptors)), behavior_sha256: behavior.digest('hex') };
}

try {
  verifyIsolation();
  result.mode = mode;
  result.css_source_sha256 = hash(fs.readFileSync(cssPath));
  if (result.css_source_sha256 !== pins[mode]) throw new Error('source-identity');
  result.before_import = process.memoryUsage();
  const started = performance.now();
  const require = createRequire(import.meta.url);
  const jsdom = require(DOM + '/node_modules/jsdom/lib/api.js');
  result.import_ms = performance.now() - started;
  result.after_import = process.memoryUsage();
  result.import_reported_rss_peak_bytes = process.resourceUsage().maxRSS * 1024;
  if (typeof jsdom.JSDOM !== 'function') throw new Error('module');
  const { CSSStyleDeclaration } = require(cssPath);
  result.compatibility = checkPrototype(CSSStyleDeclaration);
  result.after_checks = process.memoryUsage();
  if (hash(fs.readFileSync(cssPath)) !== pins[mode]) throw new Error('source-identity');
  result.accepted = true;
} catch { result.failed = true; }
fs.writeFileSync('/out/result.json', JSON.stringify(result, null, 2) + '\n',
  { flag: 'wx', mode: 0o600 });
process.exitCode = result.accepted ? 0 : 1;
