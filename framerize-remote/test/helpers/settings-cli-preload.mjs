import net from 'node:net';
import dgram from 'node:dgram';
import processes from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';

const deny = () => { throw new Error('External activity is forbidden in the settings fixture'); };
net.Socket.prototype.connect = deny;
net.Server.prototype.listen = deny;
dgram.Socket.prototype.bind = deny;
dgram.Socket.prototype.send = deny;
globalThis.fetch = deny;
for (const name of ['spawn', 'spawnSync', 'exec', 'execFile', 'execSync', 'execFileSync', 'fork']) {
  processes[name] = deny;
}
syncBuiltinESMExports();
