// Synthetic stand-in for Chrome on --remote-debugging-pipe, for browser-login tests.
// argv: mode, then the browser arguments. Records its PID and arguments in FAKE_BROWSER_STATE.
import { spawn } from 'node:child_process';
import { Socket } from 'node:net';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [mode, ...args] = process.argv.slice(2);
const profile = args.find(arg => arg.startsWith('--user-data-dir='))?.slice('--user-data-dir='.length);
writeFileSync(process.env.FAKE_BROWSER_STATE, JSON.stringify({ pid: process.pid, args }));
if (profile) writeFileSync(join(profile, 'Cookies'), 'synthetic-profile-data');
if (mode === 'exit') process.exit(0);
// A helper in the browser's process group that outlives the leader, as Chrome's renderers can.
if (mode === 'orphan' || mode === 'orphan-exit') {
  const helper = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore' });
  writeFileSync(process.env.FAKE_BROWSER_STATE, JSON.stringify({ pid: process.pid, helper: helper.pid, args }));
  if (mode === 'orphan-exit') process.exit(0);
}
if (mode === 'stubborn') {
  process.on('SIGTERM', () => {});
  // Keep this mode alive after its pipes close, until the test kills it.
  setInterval(() => {}, 1000);
}
const input = new Socket({ fd: 3, readable: true, writable: false });
const output = new Socket({ fd: 4, readable: false, writable: true });
const reply = message => output.write(`${JSON.stringify(message)}\0`);
const decoys = [
  { name: 'session', value: 'synthetic-decoy-evil', domain: 'evil.test' },
  { name: 'session', value: 'synthetic-decoy-suffix', domain: '.framer.com.evil.test' },
  { name: 'session', value: 'synthetic-decoy-lookalike', domain: '.notframer.com' },
  { name: 'session', value: 'synthetic-decoy-subdomain', domain: 'eu.framer.com' },
  { name: '__cf_bm', value: 'synthetic-decoy-cloudflare', domain: '.framer.com' },
];
const session = { name: 'session', value: 'synthetic-browser-session', domain: '.framer.com' };
let buffered = '';
input.on('data', chunk => {
  buffered += chunk.toString('utf8');
  for (let end = buffered.indexOf('\0'); end !== -1; end = buffered.indexOf('\0')) {
    const command = JSON.parse(buffered.slice(0, end));
    buffered = buffered.slice(end + 1);
    if (command.method === 'Browser.close') {
      if (mode === 'stubborn') continue;
      reply({ id: command.id, result: {} });
      output.end(() => process.exit(0));
    } else if (mode === 'malformed') output.write('not-json\0');
    else if (mode === 'oversized') output.write(Buffer.alloc(1024 * 1024 + 1, 0x20));
    else if (mode === 'unknown-id') reply({ id: 999, result: {} });
    else if (mode === 'error') reply({ id: command.id, error: { message: 'synthetic-error-text' } });
    else reply({ method: 'Target.targetCreated', params: {} }), reply({ id: command.id,
      result: { cookies: mode === 'decoys' ? decoys : [...decoys, session] } });
  }
});
// A stubborn browser keeps running after its pipe closes, until the test kills it.
input.on('end', () => { if (mode !== 'stubborn') process.exit(0); });
