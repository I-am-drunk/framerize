import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, realpath, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, win32 } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ stdin: { contents:
  "export { runClient } from './src/compatibility/command.ts'; export { planUpdate } from './src/compatibility/maintenance.ts';",
  resolveDir: fileURLToPath(new URL('../', import.meta.url)), loader: 'ts' },
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
const bundleHash = createHash('sha256').update(compiled.outputFiles[0].contents).digest('hex');
const sourceHashes = await Promise.all(Object.keys(compiled.metafile.inputs)
  .filter(path => path !== '<stdin>').sort().map(async path =>
    [path, createHash('sha256').update(await readFile(path)).digest('hex')]));
const { runClient, planUpdate } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const packages = { claude: '@anthropic-ai/claude-code', codex: '@openai/codex' };
const layouts = { backslash: 'npm\\global', unicode: 'npm-İİ', ordinary: 'npm-global' };

for (const [client, packageName] of Object.entries(packages)) {
  for (const [layout, component] of Object.entries(layouts)) {
    test(`${client} npm update preserves a ${layout} prefix`, {
      timeout: 5000, skip: process.platform === 'win32' ? 'POSIX literal-path fixture' : false,
    }, async context => {
      let root, target, cleaned = false;
      const marker = 'Synthetic client fixture, never executed.\n';
      const cleanup = async () => {
        if (cleaned) return;
        cleaned = true;
        try { if (target) assert.equal(await readFile(target, 'utf8'), marker); }
        finally {
          if (root) {
            await rm(root, { recursive: true, force: true });
            await assert.rejects(stat(root), { code: 'ENOENT' });
          }
        }
      };
      context.after(cleanup);
      root = await mkdtemp(join(tmpdir(), 'frm-npm-prefix-'));
      const home = await realpath(root);
      const prefix = join(home, component);
      const bin = join(home, 'bin');
      const executable = join(bin, client);
      const npm = join(bin, 'npm');
      const candidate = join(prefix, 'lib', 'node_modules', packageName, 'cli.js');
      await mkdir(dirname(candidate), { recursive: true, mode: 0o700 });
      await mkdir(bin, { recursive: true, mode: 0o700 });
      await writeFile(candidate, marker, { mode: 0o700 });
      target = candidate;
      await symlink(target, executable);
      const canonical = await realpath(executable);
      assert.equal(canonical, target);
      const controller = new AbortController();
      const env = { HOME: home, PATH: bin };
      const events = [];
      const output = [];
      const plan = planUpdate(client, { resolvedPath: executable,
        realPath: canonical, platform: 'linux', env });
      const code = await runClient([client, '--update', '--no-input'], {
        platform: 'linux', home, env, stateDir: join(home, 'state'),
        target: { origin: 'http://127.0.0.1:8788' }, signal: controller.signal,
        probe: path => path === executable || path === npm,
        realPath: path => path === executable ? canonical : path,
        io: { stdout: text => output.push(text), stderr: text => output.push(text) },
        async read() { throw new Error('Unexpected settings read'); },
        async fetch() { throw new Error('Unexpected release fetch'); },
        runner: {
          async capture(command, childEnv, signal) {
            assert.equal(childEnv, env);
            assert.equal(signal, controller.signal);
            if (command.command === executable) {
              assert.deepEqual(command.args, ['--version']);
              events.push({ kind: 'version' });
              return '1.0.0';
            }
            assert.equal(command.command, npm);
            events.push({ kind: 'lookup', args: command.args });
            return '"1.0.1"';
          },
          async run(command, childEnv, signal) {
            assert.equal(command.command, 'npm');
            assert.equal(childEnv, env);
            assert.equal(signal, controller.signal);
            events.push({ kind: 'update', args: command.args });
            return 0;
          },
        },
      });
      const lookup = events.find(event => event.kind === 'lookup');
      const update = events.find(event => event.kind === 'update');
      const expectedLookup = ['view', '--global', '--prefix', prefix, '--json', `${packageName}@latest`, 'version'];
      const expectedUpdate = ['install', '-g', '--prefix', prefix, `--allow-scripts=${packageName}`, `${packageName}@latest`];
      const facts = { client, layout, code, owner: plan.owner.kind,
        plannedPrefixMatches: plan.owner.prefix === prefix,
        lookupPrefixMatches: lookup?.args[3] === prefix,
        updatePrefixMatches: update?.args[3] === prefix };
      await cleanup();
      context.diagnostic(JSON.stringify({ ...facts, privateStateRemoved: true }));
      assert.equal(code, 0);
      assert.deepEqual(events.map(event => event.kind), ['version', 'lookup', 'update']);
      assert.deepEqual(plan.owner, { kind: 'npm', prefix });
      assert.deepEqual(lookup.args, expectedLookup);
      assert.deepEqual(update.args, expectedUpdate);
      assert.match(output.join(''), /1\.0\.1 is available/);
    });
  }
}

test('Windows npm prefix keeps its original spelling and root controls stay classified', context => {
  const prefix = 'C:\\Synthetic Tools\\npm';
  const target = win32.join(prefix, 'lib', 'node_modules', '@openai', 'codex', 'cli.js');
  const plan = planUpdate('codex', { resolvedPath: 'C:\\Synthetic Tools\\bin\\codex.cmd',
    realPath: target, platform: 'win32', env: {} });
  assert.equal(plan.owner.kind, 'npm');
  assert.equal(win32.normalize(plan.owner.prefix), prefix);
  const root = planUpdate('codex', { resolvedPath: '/bin/codex',
    realPath: '/lib/node_modules/@openai/codex/cli.js', platform: 'linux', env: {} });
  assert.deepEqual(root.owner, { kind: 'npm', prefix: '/' });
  const nested = planUpdate('codex', { resolvedPath: '/bin/codex',
    realPath: '/work/node_modules/other/lib/node_modules/@openai/codex/cli.js', platform: 'linux', env: {} });
  assert.equal(nested.owner.kind, 'unknown');
  context.diagnostic(JSON.stringify({ runtime: process.version, bundleHash, sourceHashes }));
});
