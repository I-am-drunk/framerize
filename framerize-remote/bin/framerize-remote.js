#!/usr/bin/env node
import { run } from '../dist/cli.js';
import { commandFailure } from '../dist/index.js';

run(process.argv.slice(2), {
  stdout: (text) => process.stdout.write(text),
  stderr: (text) => process.stderr.write(text),
}).then((code) => {
  process.exitCode = code;
}).catch((error) => {
  const diagnostic = commandFailure(error, process.argv.includes('--json'));
  if (diagnostic) {
    process.stderr.write(`${diagnostic}\n`);
    process.exitCode = 1;
    return;
  }
  const message = 'framerize-remote failed to run the requested command';
  const next = 'Check the profile and credential references. Run framerize-remote help for usage.';
  process.stderr.write(process.argv.includes('--json')
    ? `${JSON.stringify({ error: 'command_failed', message, next })}\n`
    : `${message}\n${next}\n`);
  process.exitCode = 1;
});
