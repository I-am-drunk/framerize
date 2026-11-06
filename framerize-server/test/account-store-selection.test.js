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
const { runAccountsCommand } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);

for (const action of ['remove', 'disable']) {
  for (const separate of [true, false]) {
    test(`${action} in ${separate ? 'a separate' : 'the active'} accounts file preserves selection ownership`,
      { timeout: 2000 }, async context => {
        const home = await mkdtemp(join(tmpdir(), 'frm-store-selection-'));
        context.after(() => rm(home, { recursive: true, force: true }));
        const primary = join(home, 'primary.json');
        const secondary = join(home, 'secondary.json');
        const settings = join(home, 'server.json');
        const accounts = JSON.stringify(['selected', 'other'].map(id => ({
          id, projectId: `project_${id}`, session: { env: 'SYNTHETIC_SESSION' },
        })), null, 2) + '\n';
        const before = JSON.stringify({ version: 1, config: { backend: 'framer',
          accountsFile: primary, accountId: 'selected' }, environment: {} }, null, 2) + '\n';
        await writeFile(primary, accounts, { mode: 0o600 });
        await writeFile(secondary, accounts, { mode: 0o600 });
        await writeFile(settings, before, { mode: 0o600 });
        const output = [];
        const target = separate ? secondary : primary;
        const code = await runAccountsCommand([action, 'selected', '--accounts-file', target, '--json'], {
          env: { FRAMERIZE_SERVER_HOME: home }, stdout: { write: text => output.push(text) },
          login: async () => { throw new Error('browser login must not run'); },
          readStdin: async () => { throw new Error('session input must not run'); },
        });
        const after = await readFile(settings, 'utf8');
        const changed = JSON.parse(await readFile(target, 'utf8'));
        const untouched = await readFile(separate ? primary : secondary, 'utf8');
        await rm(home, { recursive: true, force: true });
        context.diagnostic(`active selection=${JSON.parse(after).config.accountId ?? 'none'}; private state removed`);
        assert.equal(code, 0);
        assert.equal(untouched, accounts);
        assert.deepEqual(JSON.parse(output.join('')), { id: 'selected', action });
        assert.deepEqual(changed.map(account => account.id), action === 'remove' ? ['other'] : ['selected', 'other']);
        if (action === 'disable') assert.equal(changed[0].enabled, false);
        if (separate) assert.equal(after, before, 'editing a different account file must not clear the active selection');
        else {
          assert.equal(JSON.parse(after).config.accountId, undefined);
          assert.equal(JSON.parse(after).config.accountsFile, primary);
        }
      });
  }
}
