#!/usr/bin/env node
import { run } from '../dist/cli.js';
import { ServerTokenFileError } from '../dist/errors.js';

run().then((code) => { process.exitCode = code; }).catch(error => {
  const detail = error instanceof ServerTokenFileError ? {
    message: 'Server token file is unavailable or invalid.',
    next: 'Check --server-token-file, its contents and permissions. On Unix, use chmod 600.',
  } : { message: 'Server failed to start or stop' };
  process.stderr.write(`${JSON.stringify({ event: 'server.failed', ...detail })}\n`);
  process.exitCode = 1;
});
