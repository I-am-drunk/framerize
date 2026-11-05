import childProcess from 'node:child_process';
import { EventEmitter } from 'node:events';
import { readFile, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { syncBuiltinESMExports } from 'node:module';
import { join } from 'node:path';

const state = { running: false, enabled: false, calls: [] };
let server;
const save = () => writeFile(process.env.FRAMERIZE_SETUP_FIXTURE_PATH, JSON.stringify(state));
async function command(program, args) {
  state.calls.push([program, ...args]);
  if (program === 'loginctl') return { code: 0, stdout: 'yes\n' };
  if (program !== 'systemctl') throw new Error(`Unexpected command: ${program}`);
  if (args.includes('is-active')) return { code: state.running ? 0 : 1, stdout: '' };
  if (args.includes('is-enabled')) return { code: state.enabled ? 0 : 1, stdout: '' };
  if (args.includes('show')) return { code: 0, stdout: state.running
    ? `ActiveState=active\nSubState=running\nMainPID=${process.pid}\n`
    : 'ActiveState=inactive\nSubState=dead\nMainPID=0\n' };
  if (args.includes('enable')) state.enabled = true;
  if (args.includes('disable')) state.enabled = false;
  if (args.includes('restart')) {
    const saved = JSON.parse(await readFile(join(process.env.FRAMERIZE_SERVER_HOME, 'server.json')));
    server = createServer((request, response) => {
      if (request.url !== '/ready') return response.writeHead(404).end();
      process.send?.({ event: 'readiness', port: saved.config.port });
    });
    await new Promise((resolve, reject) => {
      server.once('error', reject);
      server.listen(saved.config.port, '127.0.0.1', resolve);
    });
    state.running = true;
  }
  if ((args.includes('stop') || args.includes('disable') && args.includes('--now')) && server) {
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
    server = undefined;
    state.running = false;
  }
  return { code: 0, stdout: '' };
}

childProcess.execFile = (program, args, options, callback) => {
  const child = new EventEmitter();
  child.kill = () => false;
  command(program, args).then(async result => {
    await save();
    callback(result.code ? new Error('Synthetic inactive service') : null, result.stdout);
    child.emit('close', result.code, null);
  }).catch(error => { callback(error, ''); child.emit('close', 1, null); });
  return child;
};
syncBuiltinESMExports();
process.channel?.unref();
