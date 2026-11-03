#!/usr/bin/env node
import { run } from '../src/cli.js';
import { sanitizeError } from '../src/logger.js';

run().then((code) => {
  process.exitCode = code;
}).catch((error) => {
  const message = sanitizeError(error);
  process.stderr.write(`${JSON.stringify({ event: 'fatal', message })}\n`);
  process.exitCode = 1;
});
