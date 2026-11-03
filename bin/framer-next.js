#!/usr/bin/env node
import { run } from '../src/cli.js';

run().then((code) => {
  process.exitCode = code;
}).catch((error) => {
  const message = String(error.message).replace(/(bearer\s+)\S+/gi, '$1[redacted]');
  process.stderr.write(`${JSON.stringify({ event: 'fatal', message })}\n`);
  process.exitCode = 1;
});
