import assert from 'node:assert/strict';
import http from 'node:http';
import { spawn } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { once } from 'node:events';
import { tmpdir } from 'node:os';
import path from 'node:path';

const upstream = http.createServer((request, response) => {
  response.writeHead(200, { 'content-type': 'text/event-stream' });
  response.write('data: first\n\n');
  setTimeout(() => response.end('data: [DONE]\n\n'), 20);
});
const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-smoke-'));
let child;
let childClosed;
async function waitForChild() {
  let timer;
  try {
    await Promise.race([childClosed, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('child cleanup timeout')), 5000);
    })]);
  } finally {
    clearTimeout(timer);
  }
}
try {
  upstream.listen(0, '127.0.0.1');
  await once(upstream, 'listening');
  const upstreamPort = upstream.address().port;
  const accounts = path.join(directory, 'accounts.json');
  await writeFile(accounts, JSON.stringify([{ id: 'fixture', enabled: true }]));
  child = spawn(process.execPath, ['bin/framerize-nexus.js', 'serve', '--host', '127.0.0.1',
    '--port', '0', '--upstream', `http://127.0.0.1:${upstreamPort}`, '--accounts', accounts], {
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  childClosed = new Promise((resolve) => child.once('close', resolve));
  let logs = '';
  child.stderr.on('data', (chunk) => { logs += chunk; });
  let readinessTimer;
  const port = await new Promise((resolve, reject) => {
    child.stderr.on('data', (chunk) => {
      const match = chunk.toString().match(/"port":(\d+)/);
      if (match) resolve(Number(match[1]));
    });
    child.once('exit', (code) => reject(new Error(`server exited ${code}`)));
    child.once('error', reject);
    readinessTimer = setTimeout(() => reject(new Error('readiness timeout')), 5000);
  }).finally(() => clearTimeout(readinessTimer));
  const health = await fetch(`http://127.0.0.1:${port}/health`);
  assert.equal(health.status, 200);
  const response = await fetch(`http://127.0.0.1:${port}/v1/responses`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ model: 'fixture', input: 'hello' }),
  });
  assert.equal(await response.text(), 'data: first\n\ndata: [DONE]\n\n');
  child.kill('SIGTERM');
  await waitForChild();
  assert.ok(!logs.includes('FAKE_SECRET'));
  console.log('development server smoke passed');
} finally {
  try {
    if (child) {
      if (child.pid && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
      await waitForChild();
    }
  } finally {
    try {
      upstream.closeAllConnections();
      await new Promise((resolve) => upstream.close(resolve));
    } finally {
      await rm(directory, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
    }
  }
}
