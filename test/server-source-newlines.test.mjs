import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import ts from 'typescript';

const read = file => readFileSync(new URL('../' + file, import.meta.url));

test('gitattributes keep LF text and byte-identical Castle or license inputs', () => {
  const attrs = read('.gitattributes').toString('utf8');
  assert.match(attrs, /^\* text=auto eol=lf/m);
  assert.match(attrs, /framerize-server\/src\/castle\.ts -text/);
  assert.match(attrs, /framerize-remote\/T3-LICENSE -text/);
});

test('compiler output uses LF regardless of the host platform', () => {
  const config = JSON.parse(read('tsconfig.base.json').toString());
  const converted = ts.convertCompilerOptionsFromJson(config.compilerOptions, '.');
  assert.deepEqual(converted.errors, []);
  assert.equal(converted.options.newLine, ts.NewLineKind.LineFeed);
  const output = ts.transpileModule('export const value = 1;\n', { compilerOptions: converted.options });
  assert.equal(output.outputText.includes('\r'), false);
});

test('shared TypeScript configs inherit LF emission from tsconfig.base.json', () => {
  for (const file of ['framerize-server/tsconfig.json', 'framerize-core/tsconfig.build.json',
    'tsconfig.server-tools.json', 'tsconfig.tools.json']) {
    const json = JSON.parse(read(file).toString());
    assert.match(String(json.extends), /tsconfig\.base/);
    assert.equal(json.compilerOptions?.newLine, undefined, file);
  }
});

test('Windows-style Git checkout preserves source and pinned bytes', () => {
  const root = mkdtempSync(join(tmpdir(), 'framerize-newlines-'));
  const env = { PATH: process.env.PATH, Path: process.env.Path, PATHEXT: process.env.PATHEXT,
    SystemRoot: process.env.SystemRoot, HOME: root, USERPROFILE: root,
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: join(root, 'gitconfig') };
  const git = args => execFileSync('git', args, { cwd: root, env, encoding: 'utf8' });
  const source = read('framerize-server/src/cli.ts');
  const pinned = Buffer.from('opaque\r\nasset\r\n');
  const castle = Buffer.from('castle\r\npin\r\n');
  try {
    writeFileSync(env.GIT_CONFIG_GLOBAL, '[core]\n\tautocrlf = true\n');
    git(['init', '--quiet']);
    git(['config', 'core.safecrlf', 'false']);
    mkdirSync(join(root, 'framerize-server/src'), { recursive: true });
    mkdirSync(join(root, 'framerize-remote'));
    writeFileSync(join(root, '.gitattributes'), read('.gitattributes'));
    writeFileSync(join(root, 'framerize-server/src/cli.ts'), source);
    writeFileSync(join(root, 'framerize-remote/T3-LICENSE'), pinned);
    writeFileSync(join(root, 'framerize-server/src/castle.ts'), castle);
    git(['add', '.']);
    rmSync(join(root, 'framerize-server/src/cli.ts'));
    rmSync(join(root, 'framerize-remote/T3-LICENSE'));
    rmSync(join(root, 'framerize-server/src/castle.ts'));
    git(['checkout-index', '--force', '--all']);
    const checkedOut = readFileSync(join(root, 'framerize-server/src/cli.ts'));
    assert.deepEqual(checkedOut, source);
    assert.deepEqual(readFileSync(join(root, 'framerize-remote/T3-LICENSE')), pinned);
    assert.deepEqual(readFileSync(join(root, 'framerize-server/src/castle.ts')), castle);
    assert.equal(git(['-c', 'core.autocrlf=false', 'diff', '--name-only']), '');
  } finally { rmSync(root, { recursive: true, force: true }); }
});
