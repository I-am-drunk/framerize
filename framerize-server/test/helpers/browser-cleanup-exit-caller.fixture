import { spawn } from 'node:child_process';
import { stat } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

const [bundle, root, mode] = process.argv.slice(2);
const { loginWithBrowser } = await import(pathToFileURL(bundle).href);
const controller = new AbortController();
let browser, profile;
const attempts = [];
const prefix = mode === 'graceful'
  ? "require('node:fs').createReadStream(null, { fd: 3 }).once('data', () => process.exit(0));"
  : mode === 'pipe-close' ? "require('node:fs').closeSync(4);" : '';
const script = prefix + 'setInterval(() => {}, 1000);';
try {
  await loginWithBrowser({
    interactive: true, platform: process.platform, browser: process.execPath,
    env: { HOME: root, DISPLAY: ':synthetic' }, temporaryDirectory: root,
    signal: controller.signal, timeoutMs: 10000, notice() {},
    confirm: signal => new Promise((resolve, reject) => {
      if (signal.aborted) reject(signal.reason);
      else signal.addEventListener('abort', () => reject(signal.reason), { once: true });
    }),
    spawn(command, args, options) {
      profile = args.find(value => value.startsWith('--user-data-dir=')).slice(16);
      browser = spawn(process.execPath, ['-e', script], {
        ...options, env: { HOME: root, TMPDIR: root },
      });
      browser.once('spawn', () => {
        process.send({ kind: 'browser', pid: browser.pid });
        if (mode !== 'pipe-close') controller.abort();
      });
      return browser;
    },
    kill(pid, signal) {
      if (pid !== browser.pid && pid !== -browser.pid) throw new Error('Unexpected process');
      if (signal !== 0) {
        attempts.push(signal);
        if (mode !== 'graceful') throw Object.assign(new Error('Synthetic refusal'), { code: 'EPERM' });
      }
      process.kill(pid, signal);
    },
  });
  throw new Error('Unexpected login success');
} catch (error) {
  process.exitCode = error.exitCode ?? 1;
  const removed = await stat(profile).then(() => false, error => error.code === 'ENOENT');
  await new Promise(resolve => process.send({ kind: 'outcome', code: error.code,
    attempts, profileRemoved: removed }, resolve));
  process.channel?.unref();
}
