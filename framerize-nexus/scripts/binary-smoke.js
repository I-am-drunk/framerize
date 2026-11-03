import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import http from 'node:http';

const directory = mkdtempSync(path.join(tmpdir(), 'framerize-nexus-binary-'));
const executable = path.resolve(`dist/framerize-nexus${process.platform === 'win32' ? '.exe' : ''}`);
const environment = { ...process.env, PATH: process.platform === 'win32'
  ? process.env.SystemRoot : '/usr/bin:/bin', HOME: directory };
const help = execFileSync(executable, ['--help'], {
  cwd: directory, env: environment, encoding: 'utf8',
});
assert.match(help, /Framerize Nexus/);
assert.doesNotMatch(help, /Usage: node/);
const check = execFileSync(executable, ['check'], {
  cwd: directory, env: environment, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
});
assert.match(check, /^$/);
console.log('isolated executable smoke passed');

const upstream = http.createServer((request, response) => {
  response.writeHead(200, { 'content-type': 'text/event-stream' });
  response.end('data: fixture\n\n');
});
await new Promise((resolve) => upstream.listen(0, '127.0.0.1', resolve));
const child = spawn(executable, ['serve', '--port', '0',
  '--upstream', `http://127.0.0.1:${upstream.address().port}`], {
  cwd: directory, env: environment, stdio: ['ignore', 'pipe', 'pipe'],
});
try {
  const port = await new Promise((resolve, reject) => {
    child.stderr.on('data', (chunk) => {
      const match = chunk.toString().match(/"port":(\d+)/);
      if (match) resolve(Number(match[1]));
    });
    child.once('exit', (code) => reject(new Error(`server exited ${code}`)));
    setTimeout(() => reject(new Error('readiness timeout')), 5000).unref();
  });
  const response = await fetch(`http://127.0.0.1:${port}/v1/responses`, { method: 'POST', body: '{}' });
  assert.equal(await response.text(), 'data: fixture\n\n');
  child.kill('SIGTERM');
  await new Promise((resolve) => child.once('exit', resolve));
  console.log('isolated executable server smoke passed');
} finally {
  child.kill('SIGKILL');
  upstream.close();
}
