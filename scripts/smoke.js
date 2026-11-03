import assert from 'node:assert/strict';
import http from 'node:http';
import { spawn } from 'node:child_process';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

const upstream = http.createServer((request, response) => {
  response.writeHead(200, { 'content-type': 'text/event-stream' });
  response.write('data: first\n\n');
  setTimeout(() => response.end('data: [DONE]\n\n'), 20);
});
await new Promise((resolve) => upstream.listen(0, '127.0.0.1', resolve));
const upstreamPort = upstream.address().port;
const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-smoke-'));
const accounts = path.join(directory, 'accounts.json');
await writeFile(accounts, JSON.stringify([{ id: 'fixture', enabled: true }]));
const child = spawn(process.execPath, ['bin/framer-next.js', 'serve',
  '--port', '0', '--upstream', `http://127.0.0.1:${upstreamPort}`, '--accounts', accounts], {
  stdio: ['ignore', 'pipe', 'pipe'],
});
let logs = '';
child.stderr.on('data', (chunk) => { logs += chunk; });
try {
  const port = await new Promise((resolve, reject) => {
    child.stderr.on('data', (chunk) => {
      const match = chunk.toString().match(/"port":(\d+)/);
      if (match) resolve(Number(match[1]));
    });
    child.once('exit', (code) => reject(new Error(`server exited ${code}`)));
    setTimeout(() => reject(new Error('readiness timeout')), 5000).unref();
  });
  const health = await fetch(`http://127.0.0.1:${port}/health`);
  assert.equal(health.status, 200);
  const response = await fetch(`http://127.0.0.1:${port}/v1/responses`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ model: 'fixture', input: 'hello' }),
  });
  assert.equal(await response.text(), 'data: first\n\ndata: [DONE]\n\n');
  child.kill('SIGTERM');
  await new Promise((resolve) => child.once('exit', resolve));
  assert.ok(!logs.includes('FAKE_SECRET'));
  console.log('development server smoke passed');
} finally {
  child.kill('SIGKILL');
  upstream.close();
}
