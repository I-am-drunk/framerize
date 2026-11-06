import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build } from 'esbuild';

const source = fileURLToPath(new URL('../src/compatibility/platform.ts', import.meta.url));
const compiled = await build({ entryPoints: [source], bundle: true, write: false, platform: 'node', format: 'esm' });
const bytes = compiled.outputFiles[0].contents;
const { resolveCommandPath } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
const digest = value => createHash('sha256').update(value).digest('hex');
console.log(JSON.stringify({ runtime: process.version, source: digest(await readFile(source)), bundle: digest(bytes) }));

const directory = 'C:\\synthetic\\client-bin';
const command = `${directory}\\claude.CMD`;
const executable = `${directory}\\claude.EXE`;

function locate(env, expectedPaths, platform = 'win32') {
  const original = { ...env };
  const candidates = [];
  const result = resolveCommandPath('claude', platform, Object.freeze(env), candidate => {
    candidates.push(candidate);
    return expectedPaths.includes(candidate);
  });
  assert.deepEqual(env, original);
  console.log(JSON.stringify({ keys: Object.keys(env), platform, result, candidates }));
  return result;
}

test('Windows copied mixed-case PATH still discovers an installed client', () => {
  const env = { ...{ pAtH: directory, PATHEXT: '.EXE' } };
  assert.equal(locate(env, [executable]), executable);
});

test('Windows copied mixed-case PATHEXT preserves extension preference', () => {
  const env = { ...{ PATH: directory, PathExt: '.CMD;.EXE' } };
  assert.equal(locate(env, [command, executable]), command);
});

test('Windows uppercase PATH and PATHEXT retain extension preference', () => {
  assert.equal(locate({ PATH: directory, PATHEXT: '.CMD;.EXE' }, [command, executable]), command);
});

test('Windows conventional Path casing remains supported', () => {
  assert.equal(locate({ Path: directory, PATHEXT: '.EXE' }, [executable]), executable);
});

test('POSIX lookup retains its existing executable names', () => {
  const path = '/synthetic/client-bin/claude';
  assert.equal(locate({ PATH: '/synthetic/client-bin', PathExt: '.CMD;.EXE' }, [path], 'linux'), path);
});
