import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/account-cli.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { runConfigureCommand } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);
const scenarios = [
  { name: 'explicit numeric ID is literal, not the first menu position',
    ids: ['alpha', '1'], argument: '1', selected: '1' },
  { name: 'missing numeric ID is rejected rather than selecting a menu position',
    ids: ['alpha', 'bravo'], argument: '2', reject: true },
  { name: 'explicit named ID remains selectable',
    ids: ['alpha', 'bravo'], argument: 'bravo', selected: 'bravo' },
  { name: 'interactive account number remains a menu position',
    ids: ['alpha', 'bravo'], answer: '2', selected: 'bravo' },
];

for (const scenario of scenarios) {
  test(scenario.name, { timeout: 2000 }, async context => {
    const home = await mkdtemp(join(tmpdir(), 'frm-account-id-'));
    context.after(() => rm(home, { recursive: true, force: true }));
    const accountsPath = join(home, 'accounts.json');
    const settingsPath = join(home, 'server.json');
    const accounts = JSON.stringify(scenario.ids.map(id => ({ id, projectId: `project_${id}`,
      session: { env: 'SYNTHETIC_SESSION' } })), null, 2) + '\n';
    const prior = JSON.stringify({ version: 1, config: { backend: 'http',
      upstream: 'http://127.0.0.1:9', port: 0 }, environment: {} }) + '\n';
    await writeFile(accountsPath, accounts, { mode: 0o600 });
    await writeFile(settingsPath, prior, { mode: 0o600 });
    const output = [];
    const prompts = [];
    let failure;
    let code;
    try {
      code = await runConfigureCommand(['--backend', 'framer', '--json',
        ...(scenario.argument === undefined ? [] : ['--account-id', scenario.argument])], {
        env: { FRAMERIZE_SERVER_HOME: home },
        stdout: { write: text => output.push(text) },
        ask: async question => {
          prompts.push(question);
          if (scenario.answer === undefined) throw new Error('unexpected account prompt');
          return scenario.answer;
        },
        login: async () => { throw new Error('browser login must not run'); },
        readStdin: async () => { throw new Error('session input must not run'); },
      });
    } catch (error) { failure = error; }
    const savedText = await readFile(settingsPath, 'utf8');
    const saved = JSON.parse(savedText);
    assert.equal(await readFile(accountsPath, 'utf8'), accounts);
    await rm(home, { recursive: true, force: true });
    context.diagnostic(`selected=${saved.config.accountId ?? 'none'}, failed=${Boolean(failure)}, private state removed`);
    assert.equal(prompts.length, scenario.answer === undefined ? 0 : 1);
    if (scenario.reject) {
      assert.ok(failure, 'unknown explicit numeric IDs must fail, not select a numbered account');
      assert.match(failure.message, /Unknown account id/);
      assert.equal(savedText, prior);
      assert.deepEqual(output, []);
    } else {
      assert.ifError(failure);
      assert.equal(code, 0);
      assert.equal(saved.config.backend, 'framer');
      assert.equal(saved.config.accountId, scenario.selected);
      assert.equal(JSON.parse(output.join('')).selected, scenario.selected);
    }
  });
}
