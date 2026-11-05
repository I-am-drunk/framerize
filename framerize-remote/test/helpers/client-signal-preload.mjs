import { appendFileSync } from 'node:fs';
import processes from 'node:child_process';
import net from 'node:net';
import { syncBuiltinESMExports } from 'node:module';

const record = event => appendFileSync(process.env.FRM_TEST_SIGNALS, `${JSON.stringify(event)}\n`);
const spawn = processes.spawn;
record({ kind: 'wrapper', pid: process.pid });
processes.spawn = (file, args, options) => {
  if (file !== process.env.FRM_TEST_CLIENT) throw new Error('Unexpected child command');
  const child = spawn(file, args, options), kill = child.kill.bind(child);
  record({ kind: 'child', pid: child.pid });
  child.kill = signal => { record({ kind: 'forward', signal }); return kill(signal); };
  child.once('close', (code, signal) => record({ kind: 'closed', code, signal }));
  return child;
};
net.Socket.prototype.connect = () => { throw new Error('Unexpected network connection'); };
globalThis.fetch = () => { throw new Error('Unexpected fetch'); };
syncBuiltinESMExports();
