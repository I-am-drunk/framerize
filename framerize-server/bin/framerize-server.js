#!/usr/bin/env node
import { run } from '../dist/cli.js';
import { ServerSetupError } from '../dist/service-control.js';
import { AccountFileError, AccountSelectionError, CliUsageError, ConfigurationError,
  ServerListenError, ServerTokenFileError } from '../dist/errors.js';

run().then((code) => { process.exitCode = code; }).catch(error => {
  const detail = error instanceof ServerSetupError ? {
    message: error.message,
    next: 'Run framerize-server status --json or inspect server.log in FRAMERIZE_SERVER_HOME.',
  } : error instanceof CliUsageError ? {
    message: 'Invalid command or option.',
    next: 'Run framerize-server --help for supported commands and options.',
  } : error instanceof ServerTokenFileError ? {
    message: 'Server token file is unavailable or invalid.',
    next: 'Check --server-token-file, its contents and permissions. On Unix, use chmod 600.',
  } : error instanceof AccountFileError ? {
    message: 'Account metadata file is unavailable or invalid.',
    next: 'Check --accounts-file, read permissions and the documented JSON account format.',
  } : error instanceof AccountSelectionError ? {
    message: 'Select one configured Framer account.',
    next: 'Check --account-id or FRAMERIZE_SERVER_ACCOUNT and --accounts-file or FRAMERIZE_SERVER_ACCOUNTS. Select one enabled account with projectId and a session reference.',
  } : error instanceof ServerListenError ? {
    message: 'Server could not listen on the configured host and port.',
    next: 'Check --host or FRAMERIZE_SERVER_HOST and --port or FRAMERIZE_SERVER_PORT. Use an available local address and port.',
  } : error instanceof ConfigurationError ? {
    message: 'Server configuration is invalid.',
    next: 'Check option values, FRAMERIZE_SERVER_* settings and account session references. Run framerize-server --help for options. Environment guide: https://github.com/I-am-drunk/framerize/blob/main/framerize-server/docs/environment.md.',
  } : { message: 'Server failed to start or stop' };
  process.stderr.write(`${JSON.stringify({ event: 'server.failed', ...detail })}\n`);
  process.exitCode = 1;
});
