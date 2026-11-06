import fs from 'node:fs/promises';
import { appendFileSync } from 'node:fs';
import net from 'node:net';
import processes from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';

const open = fs.open, spawn = processes.spawn;
fs.open = async (path, ...args) => {
  if (String(path).endsWith('/managed-settings.json')) {
    throw Object.assign(new Error('No synthetic managed settings'), { code: 'ENOENT' });
  }
  return open(path, ...args);
};
processes.spawn = (file, args, options) => {
  if (file !== process.env.FRM_TEST_CLIENT) throw new Error('Unexpected child command');
  return spawn(file, args, options);
};
net.Socket.prototype.connect = () => { throw new Error('Unexpected network connection'); };
globalThis.fetch = async url => {
  appendFileSync(process.env.FRM_TEST_FETCHES, `${JSON.stringify(String(url))}\n`);
  if (!/^https:\/\/downloads\.claude\.ai\/claude-code-releases\/(latest|stable)$/.test(String(url))) {
    throw new Error('Unexpected release URL');
  }
  return new Response('2.1.300');
};
syncBuiltinESMExports();
