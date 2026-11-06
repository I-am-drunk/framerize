import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build, version } from 'esbuild';

const compiled = await build({
  stdin: { contents: "export { runAccountsCommand } from './src/account-cli.ts'; export { loadAccounts } from './src/accounts.ts';",
    resolveDir: fileURLToPath(new URL('../', import.meta.url)), loader: 'ts' },
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const bytes = compiled.outputFiles[0].contents;
const { runAccountsCommand, loadAccounts } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
const digest = createHash('sha256').update(bytes).digest('hex');

for (const count of [2, 9000]) {
  test(`editing ${count} compact accounts cannot leave an unreadable store`,
    { timeout: 5000 }, async context => {
      const home = await mkdtemp(join(tmpdir(), 'frm-account-write-bound-'));
      context.after(() => rm(home, { recursive: true, force: true }));
      const file = join(home, 'accounts.json');
      const accounts = Array.from({ length: count }, (_, index) => ({
        id: `account_${index}`, projectId: 'synthetic', session: { env: 'SYNTHETIC_SESSION' },
      }));
      const before = JSON.stringify(accounts);
      const expanded = JSON.stringify([{ ...accounts[0], enabled: false }, ...accounts.slice(1)], null, 2) + '\n';
      assert.ok(Buffer.byteLength(before) < 1024 * 1024);
      assert.equal(Buffer.byteLength(expanded) > 1024 * 1024, count === 9000);
      await writeFile(file, before, { mode: 0o600 });
      assert.equal((await loadAccounts(file)).length, count);
      let forbiddenCalls = 0;
      const forbidden = async () => { forbiddenCalls++; throw new Error('Unexpected input request'); };
      const output = [];
      let code;
      let failure;
      try {
        code = await runAccountsCommand(['disable', 'account_0', '--json'], {
          env: { FRAMERIZE_SERVER_HOME: home }, stdout: { write: text => output.push(text) },
          login: forbidden, readStdin: forbidden, ask: forbidden, askHidden: forbidden,
        });
      } catch (error) { failure = { name: error.name, message: error.message }; }
      const after = await readFile(file, 'utf8');
      let recovered;
      let readFailure;
      try { recovered = await loadAccounts(file); }
      catch (error) { readFailure = { name: error.name, message: error.message }; }
      const entries = await readdir(home);
      await rm(home, { recursive: true, force: true });
      context.diagnostic(JSON.stringify({ node: process.version, esbuild: version, compiledSha256: digest,
        count, beforeBytes: Buffer.byteLength(before), afterBytes: Buffer.byteLength(after),
        code, failure, readFailure, output: output.join(''), privateStateRemoved: true }));
      assert.equal(forbiddenCalls, 0);
      assert.deepEqual(entries, ['accounts.json']);
      assert.equal(readFailure, undefined, 'a completed account edit must leave a readable store');
      assert.equal(recovered.length, count);
      if (failure) {
        assert.equal(count, 9000, 'the small control must still succeed');
        assert.equal(after, before, 'a rejected edit must preserve exact prior bytes');
        assert.equal(output.join(''), '', 'a rejected edit must not report success');
      } else {
        assert.equal(code, 0);
        assert.equal(recovered[0].enabled, false);
        assert.deepEqual(JSON.parse(output.join('')), { id: 'account_0', action: 'disable' });
      }
    });
}
