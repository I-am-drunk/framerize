import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createConnection } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const sourceRoot = resolve(process.env.FRAMERIZE_REMOTE_SOURCE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const compiled = await build({ entryPoints: [join(sourceRoot, 'src/compatibility/index.ts')],
  bundle: true, platform: 'node', format: 'esm', write: false });
const runtime = compiled.outputFiles[0].contents;
console.log(`Source bundle SHA256 ${createHash('sha256').update(runtime).digest('hex')}`);

async function bounded(promise, message, milliseconds = 2000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

function portState(port) {
  return new Promise(resolve => {
    const socket = createConnection({ host: '127.0.0.1', port });
    const finish = result => { socket.destroy(); resolve(result); };
    socket.once('connect', () => finish('open'));
    socket.once('error', error => finish(error.code));
    socket.setTimeout(300, () => finish('timeout'));
  });
}

const wrapperSource = `
import { spawn } from 'node:child_process';
import { nodeRunner, runClient } from './runtime.mjs';
const runner = nodeRunner(process.platform, (...args) => {
  const child = spawn(...args);
  process.send({ kind: 'client', pid: child.pid });
  child.once('close', (code, signal) => process.send({ kind: 'client-close', code, signal }));
  return child;
});
process.exitCode = await runClient(['codex', '--no-input'], {
  platform: process.platform, env: process.env, home: process.env.HOME,
  stateDir: process.env.HOME, target: { origin: 'http://127.0.0.1:8788' }, runner,
  io: { stdout: text => process.stdout.write(text), stderr: text => process.stderr.write(text) },
});
process.disconnect();
`;

for (const signal of ['SIGINT', 'SIGTERM']) {
  test(`ordinary client launch forwards wrapper-only ${signal} without a terminal`,
    { skip: process.platform === 'win32', timeout: 6000 }, async context => {
      const directory = await mkdtemp(join(tmpdir(), 'frm-client-launch-signal-'));
      let wrapper, closed, clientPid, clientClosed = false, port;
      context.after(async () => {
        try {
          if (clientPid && !clientClosed) {
            try { process.kill(clientPid, 'SIGKILL'); }
            catch (error) { if (error.code !== 'ESRCH') throw error; }
          }
          if (wrapper && wrapper.exitCode === null && wrapper.signalCode === null) wrapper.kill('SIGTERM');
          if (closed) {
            try { await bounded(closed, 'Owned wrapper cleanup timed out'); }
            catch {
              wrapper.kill('SIGKILL');
              await bounded(closed, 'Forced wrapper cleanup timed out');
            }
          }
          if (port) assert.equal(await portState(port), 'ECONNREFUSED');
          context.diagnostic(JSON.stringify({ cleanup: 'complete', clientClosed, portRefused: Boolean(port) }));
        } finally { await rm(directory, { recursive: true, force: true }); }
      });
      const bin = join(directory, 'bin');
      await mkdir(bin);
      await writeFile(join(directory, 'runtime.mjs'), runtime);
      await writeFile(join(directory, 'wrapper.mjs'), wrapperSource);
      const profile = join(directory, 'profile.json'), original = '{"synthetic":"unchanged"}\n';
      await writeFile(profile, original);
      await writeFile(join(bin, 'codex'), `#!${process.execPath}
const listener = require('node:net').createServer(socket => socket.end());
listener.listen(0, '127.0.0.1', () => {
  console.log(JSON.stringify({ kind: 'ready', pid: process.pid, port: listener.address().port }));
});
`, { mode: 0o700 });
      const ready = Promise.withResolvers();
      let stdout = '', stderr = '', pendingLine = '', clientExit;
      wrapper = spawn(process.execPath, [join(directory, 'wrapper.mjs')], {
        cwd: directory, detached: true, stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
        env: { HOME: directory, USERPROFILE: directory, PATH: bin,
          TMPDIR: directory, TMP: directory, TEMP: directory, NODE_DISABLE_COMPILE_CACHE: '1' },
      });
      closed = new Promise((resolve, reject) => {
        wrapper.once('error', reject);
        wrapper.once('close', (code, signal) => resolve({ code, signal }));
      });
      wrapper.on('message', message => {
        if (message.kind === 'client') clientPid = message.pid;
        if (message.kind === 'client-close') { clientClosed = true; clientExit = message; }
      });
      wrapper.stdout.setEncoding('utf8').on('data', chunk => {
        stdout += chunk;
        pendingLine += chunk;
        const lines = pendingLine.split('\n');
        pendingLine = lines.pop();
        for (const line of lines) {
          try { const value = JSON.parse(line); if (value.kind === 'ready') ready.resolve(value); }
          catch {}
        }
      });
      wrapper.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
      const state = await bounded(Promise.race([ready.promise, closed.then(exit => {
        throw new Error(`Wrapper exited before client readiness: ${JSON.stringify(exit)} ${stderr}`);
      })]), 'Synthetic client did not listen');
      port = state.port;
      assert.equal(state.pid, clientPid);
      assert.equal(await portState(port), 'open');
      assert.equal(wrapper.kill(signal), true);
      const observation = await bounded(closed, 'still-running', 1000)
        .catch(error => ({ pending: true, reason: error.message }));
      const afterSignal = await portState(port);
      assert.equal(await readFile(profile, 'utf8'), original);
      context.diagnostic(JSON.stringify({ signal, observation, afterSignal,
        clientClosed, clientExit, stderr }));
      assert.deepEqual(observation, { code: 1, signal: null });
      assert.equal(clientClosed, true);
      assert.equal(afterSignal, 'ECONNREFUSED');
      assert.ok(stdout.includes('"kind":"ready"'));
    });
}
