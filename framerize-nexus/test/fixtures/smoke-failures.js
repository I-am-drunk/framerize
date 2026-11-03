import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import filesystem from 'node:fs/promises';
import http from 'node:http';
import { syncBuiltinESMExports } from 'node:module';
import path from 'node:path';

if (process.env.NEXUS_SMOKE_FAILURE) {
  const failure = process.env.NEXUS_SMOKE_FAILURE;
  const servers = [];
  const createServer = http.createServer;
  http.createServer = (...args) => {
    const server = createServer(...args);
    const state = { started: false, closed: false };
    servers.push(state);
    server.once('listening', () => { state.started = true; });
    server.once('close', () => { state.closed = true; });
    return server;
  };
  const listen = http.Server.prototype.listen;
  let listens = 0;
  http.Server.prototype.listen = function (...args) {
    listens += 1;
    if ((failure === 'listen' && listens === 1) || (failure === 'app-listen' && listens === 2)) {
      queueMicrotask(() => this.emit('error', new Error('injected listen failure')));
      return this;
    }
    return listen.apply(this, args);
  };
  const children = [];
  const spawn = childProcess.spawn;
  childProcess.spawn = (executable, args, options) => {
    if (failure === 'spawn') executable = path.join(process.env.HOME, 'missing-executable');
    if (failure === 'early-exit') args = ['-e', 'process.exit(12)'];
    if (failure === 'startup-timeout') args = ['-e', 'setInterval(() => {}, 1000)'];
    const child = spawn(executable, args, options);
    const state = { closed: false };
    children.push(state);
    child.once('close', () => { state.closed = true; });
    return child;
  };
  const writeFile = filesystem.writeFile;
  filesystem.writeFile = (file, content, ...args) => {
    if (failure === 'write') throw new Error('injected write failure');
    if (failure === 'invalid-config') content = '{invalid';
    return writeFile(file, content, ...args);
  };
  if (failure === 'fetch') globalThis.fetch = async () => { throw new Error('injected fetch failure'); };
  const remove = filesystem.rm;
  filesystem.rm = async (directory, options) => {
    assert.equal(path.dirname(directory), process.env.TMPDIR);
    assert.ok(children.every((child) => child.closed), 'child must close before removal');
    assert.ok(servers.every((server) => !server.started || server.closed), 'server must close before removal');
    await remove(directory, options);
    await writeFile(process.env.NEXUS_SMOKE_REPORT, JSON.stringify({
      directory, children, servers,
      environmentRestored: process.env.FRAMER_SMOKE_JWT === 'saved-fixture-jwt'
        && process.env.FRAMER_SMOKE_CASTLE === 'saved-fixture-castle',
    }));
  };
  syncBuiltinESMExports();
}
