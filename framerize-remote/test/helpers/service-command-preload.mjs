import processes from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';

const rollbackFailure = process.env.FRM_FIXTURE_ROLLBACK_FAILURE === '1';
const deniedGroups = new Set();

const worker = `import { createServer } from 'node:net';
  process.on('SIGTERM', () => {});
  const server = createServer(socket => socket.end());
  server.listen(0, '127.0.0.1', () => {
    process.send({ workerPid: process.pid, port: server.address().port });
    process.disconnect();
  });`;
const command = `import { spawn } from 'node:child_process';
  const rollback = ${rollbackFailure};
  if (process.argv[1] === 'loginctl') console.log('yes');
  else if (!(rollback ? process.argv.includes('restart') || process.argv.includes('disable')
    : process.argv.includes('show-environment'))) process.exit(0);
  else {
    process.on('SIGTERM', () => process.exit(0));
    const child = spawn(process.execPath, ['--input-type=module', '-e', ${JSON.stringify(worker)}],
      { stdio: ['ignore', 'ignore', 'ignore', 'ipc'] });
    child.once('message', value => {
      const line = JSON.stringify({ kind: 'ready', pid: process.pid, ...value }) + String.fromCharCode(10);
      process.stdout.write(line, () => {
        if (rollback && process.argv.includes('disable')) process.exit(1);
      });
    });
  }`;
const originalSpawn = processes.spawn, originalExec = processes.execFile;
const argsFor = (program, args) => {
  if (!['systemctl', 'loginctl'].includes(program)) throw new Error(`Forbidden command: ${program}`);
  return ['--input-type=module', '-e', command, program, ...args];
};
function observe(child) {
  process.send?.({ kind: 'child', pid: child.pid });
  let output = '';
  child.stdout.on('data', chunk => {
    output += chunk.toString('utf8');
    if (output.startsWith('{') && output.includes('\n')) {
      process.send?.(JSON.parse(output.trim()));
      output = '';
    }
  });
  return child;
}
processes.spawn = (program, args, options) => {
  const child = observe(originalSpawn(process.execPath, argsFor(program, args), options));
  if (rollbackFailure && args.includes('disable')) deniedGroups.add(child.pid);
  return child;
};
processes.execFile = (program, args, options, callback) => {
  if (program === '/bin/ps') return originalExec(program, args, options, callback);
  return observe(originalExec(process.execPath, argsFor(program, args), options, callback));
};
for (const name of ['spawnSync', 'exec', 'execSync', 'execFileSync', 'fork']) {
  processes[name] = () => { throw new Error(`Forbidden process API: ${name}`); };
}
syncBuiltinESMExports();
if (process.env.FRM_FIXTURE_DENY_STOP === '1' || rollbackFailure) {
  const kill = process.kill;
  process.kill = (pid, signal) => {
    if (pid < 0 && (!rollbackFailure || deniedGroups.has(-pid))) {
      throw Object.assign(new Error('synthetic-private-denial'), { code: 'EPERM' });
    }
    return kill(pid, signal);
  };
}
process.channel?.unref();
