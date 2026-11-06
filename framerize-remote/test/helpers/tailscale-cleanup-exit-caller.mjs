import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const [bundle, receipt, platform, mode] = process.argv.slice(2);
const { readTailscaleStatus } = await import(pathToFileURL(bundle).href);
const controller = new AbortController();
const originalKill = process.kill;
const finish = (result, code) => {
  process.exitCode = code;
  process.send(result, () => process.disconnect());
};
try {
  const result = await readTailscaleStatus({ platform, signal: controller.signal, env: {},
    launch(command, args, options) {
      const child = spawn(process.execPath, ['-e', 'console.log("ready"); setInterval(() => {}, 1000);'], options);
      try { writeFileSync(receipt, JSON.stringify({ pid: child.pid, group: options.detached })); }
      catch (error) { child.kill('SIGKILL'); throw error; }
      if (mode === 'deny' && platform === 'win32') child.kill = () => false;
      if (mode === 'deny' && platform !== 'win32') process.kill = (pid, signal) => {
        if (pid === -child.pid) throw Object.assign(new Error('Synthetic signal denial'), { code: 'EACCES' });
        return originalKill(pid, signal);
      };
      child.stdout.once('data', () => controller.abort());
      return child;
    },
  });
  finish({ state: result.state }, 0);
} catch (error) {
  finish({ error: error.code }, 1);
}
