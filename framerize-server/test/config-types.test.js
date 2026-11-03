import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

test('compiled configuration and account declarations preserve validated types', { timeout: 30000 }, (context) => {
  const root = mkdtempSync(path.join(tmpdir(), 'server-config-types-'));
  context.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(path.join(root, 'types'));
  const source = fileURLToPath(new URL('../dist/config.js', import.meta.url)).replaceAll('\\', '/');
  const accountsSource = fileURLToPath(new URL('../dist/accounts.js', import.meta.url)).replaceAll('\\', '/');
  const compiler = fileURLToPath(new URL('../../node_modules/typescript/bin/tsc', import.meta.url));
  const file = path.join(root, 'consumer.mts');
  const valid = `import { loadConfig, readServerToken } from ${JSON.stringify(source)};
const config = loadConfig({ port: '8789' }, {});
const port: number = config.port;
const token: string = config.serverToken;
const loaded: Promise<string> = readServerToken(config);
import { loadAccounts } from ${JSON.stringify(accountsSource)};
const accounts = await loadAccounts();
const account = accounts[0]!;
const id: string = account.id;
const models: readonly string[] | undefined = account.models;`;
  const check = (extra = '') => {
    writeFileSync(file, `${valid}\n${extra}\n`);
    return spawnSync(process.execPath, [compiler, '--noEmit', '--strict', '--skipLibCheck',
      '--target', 'ES2022', '--module', 'NodeNext', '--typeRoots', path.join(root, 'types'), file],
    { cwd: root, encoding: 'utf8', timeout: 10000 });
  };
  const positive = check();
  assert.equal(positive.status, 0, `${positive.stdout}\n${positive.stderr}`);
  for (const [extra, diagnostic] of [
    ['config.port = 1;', /TS2540/],
    ['const invalid: string = config.port;', /TS2322/],
    ['account.id = "other";', /TS2540/],
    ['accounts.push(account);', /TS2339/],
    ['account.models?.push("other");', /TS2339/],
    ['account.authorization;', /TS2339/],
    ['loadAccounts(42);', /TS2345/],
  ]) {
    const negative = check(extra);
    assert.equal(negative.status, 2, `${negative.stdout}\n${negative.stderr}`);
    assert.match(negative.stdout, diagnostic);
  }
});
