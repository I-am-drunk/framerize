import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/install.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { installCommand } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function optionalText(path) {
  try { return await readFile(path, 'utf8'); }
  catch (error) { if (error.code === 'ENOENT') return null; throw error; }
}

const curlStub = `#!${process.execPath}
const { writeFileSync } = require('node:fs');
writeFileSync(process.env.FIXTURE_CURL_LOG, JSON.stringify(process.argv.slice(2)));
process.stdout.write(process.env.FIXTURE_SCRIPT, () => {
  process.exitCode = Number(process.env.FIXTURE_CURL_EXIT);
});
`;

async function execute(context, command, root, env) {
  let child;
  const result = new Promise(resolve => {
    child = execFile(command.command, command.args, { cwd: root, env,
      timeout: 3000, killSignal: 'SIGKILL', maxBuffer: 16384 }, (error, stdout, stderr) => {
      resolve({ code: error?.code ?? 0, signal: error?.signal ?? null, stdout, stderr });
    });
  });
  const closed = new Promise(resolve => child.once('close', resolve));
  context.after(async () => {
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    await closed;
  });
  return result;
}

const modes = [
  { name: 'empty failed download', download: 22, script: null, expected: 22, runs: false },
  { name: 'partial failed download', download: 18, script: 0, expected: 18, runs: false },
  { name: 'completed installer', download: 0, script: 0, expected: 0, runs: true },
  { name: 'installer failure', download: 0, script: 23, expected: 23, runs: true },
];

for (const client of ['claude', 'codex']) for (const mode of modes) {
  test(`${client}: ${mode.name}`, { timeout: 6000,
    skip: process.platform === 'win32' ? 'POSIX installer shell commands' : false }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'frm-installer-download-'));
    context.after(() => rm(root, { recursive: true, force: true }));
    const bin = join(root, 'bin');
    const temporary = join(root, 'temporary with spaces');
    const marker = join(root, 'executed');
    const log = join(root, 'curl-arguments');
    await mkdir(bin);
    await mkdir(temporary);
    await writeFile(join(bin, 'curl'), curlStub, { mode: 0o700 });
    const script = mode.script === null ? ''
      : `printf ran > "$FIXTURE_MARKER"\nexit ${mode.script}\n`;
    const env = { HOME: root, TMPDIR: temporary, PATH: `${bin}:/usr/bin:/bin`,
      FIXTURE_MARKER: marker, FIXTURE_CURL_LOG: log,
      FIXTURE_CURL_EXIT: String(mode.download), FIXTURE_SCRIPT: script };
    const command = installCommand(client, 'linux');
    assert.notEqual(command, null);
    const result = await execute(context, command, root, env);
    const execution = await optionalText(marker);
    assert.deepEqual({ code: result.code, execution },
      { code: mode.expected, execution: mode.runs ? 'ran' : null });
    assert.equal(result.signal, null);
    assert.equal(result.stdout, '');
    assert.equal(result.stderr, '');
    const url = client === 'claude' ? 'https://claude.ai/install.sh' : 'https://chatgpt.com/codex/install.sh';
    assert.deepEqual(JSON.parse(await readFile(log, 'utf8')), ['-fsSL', url]);
    assert.deepEqual(await readdir(temporary), [], 'installer temporary files must be removed');
  });
}
