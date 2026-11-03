import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const files = [
  'test/config.test.js',
  'test/accounts.test.js',
  'test/logger.test.js',
  'test/routes.test.js',
  'test/streaming.test.js',
  'test/shutdown.test.js',
];
const child = spawn(process.execPath, ['--test', '--test-reporter=spec', ...files], {
  cwd: root,
  stdio: 'inherit',
  env: { ...process.env, NODE_NO_WARNINGS: '1' },
});
child.once('error', (error) => {
  console.error(`Acceptance runner failed: ${error.message}`);
  process.exitCode = 1;
});
child.once('exit', (code, signal) => {
  process.exitCode = code ?? (signal ? 1 : 0);
});
