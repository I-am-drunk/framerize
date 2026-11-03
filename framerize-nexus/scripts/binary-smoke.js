import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import http from 'node:http';

const directory = mkdtempSync(path.join(tmpdir(), 'framerize-nexus-binary-'));
const name = `framerize-nexus${process.platform === 'win32' ? '.exe' : ''}`;
let executable = path.resolve(`dist/${name}`);
if (process.argv[2]) {
  const archive = path.resolve(process.argv[2]);
  if (process.platform === 'win32') {
    execFileSync('powershell', ['-NoLogo', '-NoProfile', '-Command',
      `Expand-Archive -LiteralPath '${archive}' -DestinationPath '${directory}' -Force`],
    { stdio: 'inherit' });
  } else {
    execFileSync('tar', ['-xzf', archive, '-C', directory], { stdio: 'inherit' });
  }
  executable = path.join(directory, name);
}
for (const license of ['LICENSE', 'NODE-LICENSE']) {
  const file = path.join(path.dirname(executable), license);
  if (process.argv[2]) assert.ok(existsSync(file));
  assert.ok(readFileSync(file, 'utf8').length > 0);
}
const environment = { PATH: process.platform === 'win32'
  ? process.env.SystemRoot : '', HOME: directory, SystemRoot: process.env.SystemRoot };
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
  '--host', '127.0.0.1', '--upstream', `http://127.0.0.1:${upstream.address().port}`], {
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
