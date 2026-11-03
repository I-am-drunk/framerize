#!/usr/bin/env node
import { run } from '../src/cli.js';

run().then((code) => { process.exitCode = code; }).catch(() => {
  process.stderr.write(`${JSON.stringify({ event: 'server.failed', message: 'Server failed to start or stop' })}\n`);
  process.exitCode = 1;
});
