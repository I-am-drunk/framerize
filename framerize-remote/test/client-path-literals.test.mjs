import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/platform.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { resolveCommandPath } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function run(context, command, cwd, env) {
  let child;
  const output = new Promise((resolve, reject) => {
    child = execFile(command, ['--version'], { cwd, env, timeout: 2000,
      killSignal: 'SIGKILL', maxBuffer: 8192 }, (error, stdout, stderr) => {
      if (error) reject(error);
      else resolve({ stdout, stderr });
    });
  });
  const closed = new Promise(resolve => child.once('close', resolve));
  context.after(async () => {
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    await closed;
  });
  return output;
}

for (const [label, suffix] of [['trailing whitespace', ' '], ['literal quote', '"'], ['ordinary name', '-ordinary']]) {
  test(`POSIX PATH preserves a directory's ${label}`, { timeout: 6000,
    skip: process.platform === 'win32' ? 'POSIX executable lookup and path syntax' : false }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'frm-path-literal-'));
    context.after(() => rm(root, { recursive: true, force: true }));
    const directory = join(root, `bin${suffix}`);
    const decoyDirectory = join(root, 'bin');
    await mkdir(directory);
    await mkdir(decoyDirectory);
    for (const [location, marker] of [[directory, 'intended'], [decoyDirectory, 'decoy']]) {
      await writeFile(join(location, 'claude'),
        `#!${process.execPath}\nprocess.stdout.write(${JSON.stringify(`${marker}\n`)});\n`, { mode: 0o700 });
    }
    const env = { PATH: directory, HOME: root, TMPDIR: root, NODE_DISABLE_COMPILE_CACHE: '1' };
    const expected = await run(context, 'claude', root, env);
    assert.deepEqual(expected, { stdout: 'intended\n', stderr: '' });
    const executable = resolveCommandPath('claude', process.platform, env);
    assert.equal(typeof executable, 'string');
    const actual = await run(context, executable, root, env);
    assert.deepEqual(actual, expected, 'the resolver must select the executable that POSIX PATH selects');
    assert.equal(executable, join(directory, 'claude'));
    assert.deepEqual(env, { PATH: directory, HOME: root, TMPDIR: root, NODE_DISABLE_COMPILE_CACHE: '1' });
  });
}

test('Windows retains quoted PATH and PATHEXT handling', () => {
  const checked = [];
  const expected = 'C:\\Client Tools\\claude.EXE';
  const found = resolveCommandPath('claude', 'win32', {
    PATH: '  "C:\\Client Tools"  ;C:\\unused', PATHEXT: '.EXE;.CMD',
  }, (candidate, platform) => {
    checked.push(candidate);
    assert.equal(platform, 'win32');
    return candidate === expected;
  });
  assert.equal(found, expected);
  assert.deepEqual(checked, [expected]);
});

test('POSIX keeps directory order and skips empty or relative entries', () => {
  const checked = [];
  const found = resolveCommandPath('claude', 'linux', { PATH: ':relative:/first:/second' }, candidate => {
    checked.push(candidate);
    return candidate === '/second/claude';
  });
  assert.equal(found, '/second/claude');
  assert.deepEqual(checked, ['/first/claude', '/second/claude']);
});
