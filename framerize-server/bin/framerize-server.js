#!/usr/bin/env node
import { run } from '../dist/cli.js';
import { AccountFileError, AccountSelectionError, ServerTokenFileError } from '../dist/errors.js';

run().then((code) => { process.exitCode = code; }).catch(error => {
  const detail = error instanceof ServerTokenFileError ? {
    message: 'Server token file is unavailable or invalid.',
    next: 'Check --server-token-file, its contents and permissions. On Unix, use chmod 600.',
  } : error instanceof AccountFileError ? {
    message: 'Account metadata file is unavailable or invalid.',
    next: 'Check --accounts-file, read permissions and the documented JSON account format.',
  } : error instanceof AccountSelectionError ? {
    message: 'Select one configured Framer account.',
    next: 'Check --account-id or FRAMERIZE_SERVER_ACCOUNT and --accounts-file or FRAMERIZE_SERVER_ACCOUNTS. Select one enabled account with projectId and a session reference.',
  } : { message: 'Server failed to start or stop' };
  process.stderr.write(`${JSON.stringify({ event: 'server.failed', ...detail })}\n`);
  process.exitCode = 1;
});
