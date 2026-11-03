import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { loadAccounts, readCredential } from '../src/config.js';
import net from 'node:net';
import { mintCredential } from '../src/credentials.js';

function socketAddress(directory) {
  return process.platform === 'win32'
    ? `\\\\.\\pipe\\framer-next-${path.basename(directory)}`
    : path.join(directory, 'mint.sock');
}

test('private credential files are read without JSON secrets', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-credentials-'));
  const credential = path.join(directory, 'session.private');
  await writeFile(credential, 'session=fixture-secret\n');
  assert.equal(await readCredential(credential), 'session=fixture-secret');
  const accountsFile = path.join(directory, 'accounts.json');
  await writeFile(accountsFile, JSON.stringify([{ id: 'one', provider: 'framer',
    projectId: 'project', sessionFile: credential, castleFile: credential }]));
  const [account] = await loadAccounts(accountsFile);
  assert.equal(account.sessionFile, credential);
  assert.equal(account.castleFile, credential);
  assert.equal(JSON.stringify(account).includes('fixture-secret'), false);
});

test('invalid credential references fail safely', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-credentials-'));
  const nested = path.join(directory, 'nested');
  await mkdir(nested);
  await assert.rejects(readCredential(nested), /small regular file/);
  const oversized = path.join(directory, 'oversized');
  await writeFile(oversized, 'x'.repeat(16 * 1024 + 1));
  await assert.rejects(readCredential(oversized), /small regular file/);
  await assert.rejects(readCredential(path.join(directory, 'missing')));
});

test('credential service returns a fresh token without logging it', async (t) => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-socket-'));
  const socketPath = socketAddress(directory);
  const server = net.createServer((socket) => {
    socket.once('data', () => socket.end(`${JSON.stringify({ ok: true, token: 'fresh-fixture' })}\n`));
  });
  await new Promise((resolve) => server.listen(socketPath, resolve));
  t.after(() => new Promise((resolve) => server.close(resolve)));
  assert.equal(await mintCredential(socketPath, 'fixture'), 'fresh-fixture');
});

test('credential service response can arrive after the request', async (t) => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-socket-'));
  const socketPath = socketAddress(directory);
  const server = net.createServer((socket) => {
    socket.once('data', () => setTimeout(() => {
      socket.write(`${JSON.stringify({ ok: true, token: 'delayed-fixture' })}\n`);
    }, 20));
  });
  await new Promise((resolve) => server.listen(socketPath, resolve));
  t.after(() => new Promise((resolve) => server.close(resolve)));
  assert.equal(await mintCredential(socketPath, 'fixture'), 'delayed-fixture');
});

test('credential service EOF fails without waiting for timeout', async (t) => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-socket-'));
  const socketPath = socketAddress(directory);
  const sockets = new Set();
  const server = net.createServer((socket) => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    socket.end();
  });
  await new Promise((resolve) => server.listen(socketPath, resolve));
  t.after(() => new Promise((resolve) => {
    for (const socket of sockets) socket.destroy();
    server.close(resolve);
  }));
  await assert.rejects(mintCredential(socketPath, 'fixture'), /closed without a response/);
});

test('credential service rejects oversized responses', async (t) => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-socket-'));
  const socketPath = socketAddress(directory);
  const sockets = new Set();
  const server = net.createServer((socket) => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    socket.write('x'.repeat(16 * 1024 + 1));
  });
  await new Promise((resolve) => server.listen(socketPath, resolve));
  t.after(() => new Promise((resolve) => {
    for (const socket of sockets) socket.destroy();
    server.close(resolve);
  }));
  await assert.rejects(mintCredential(socketPath, 'fixture'), /response too large/);
});

test('credential service rejects unsuccessful responses', async (t) => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-socket-'));
  const socketPath = socketAddress(directory);
  const sockets = new Set();
  const server = net.createServer((socket) => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    socket.end('{"ok":false}\n');
  });
  await new Promise((resolve) => server.listen(socketPath, resolve));
  t.after(() => new Promise((resolve) => {
    for (const socket of sockets) socket.destroy();
    server.close(resolve);
  }));
  await assert.rejects(mintCredential(socketPath, 'fixture'), /could not mint/);
});
