import processes from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';

const spawn = processes.spawn;
processes.spawn = (command, args, options) => {
  if (!['systemctl', 'loginctl'].includes(command)) {
    throw new Error(`Forbidden service fixture command: ${command}`);
  }
  return spawn(process.execPath, ['-e',
    "process.stdout.write(process.argv[1] === 'loginctl' ? 'yes\\n' : '')", command], options);
};
for (const name of ['exec', 'execFile', 'execSync', 'execFileSync', 'spawnSync', 'fork']) {
  processes[name] = () => { throw new Error(`Forbidden process API: ${name}`); };
}
globalThis.fetch = () => { throw new Error('Network is forbidden in the service fixture'); };
syncBuiltinESMExports();
