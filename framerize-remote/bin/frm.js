#!/usr/bin/env node
import { createInterface } from 'node:readline/promises';
import { runFrm, commandFailure } from '../dist/index.js';

const json = process.argv.includes('--json');
const terminal = Boolean(process.stdin.isTTY && process.stdout.isTTY);
const ask = terminal ? async (question, signal) => {
  const prompt = createInterface({ input: process.stdin, output: process.stdout });
  const controller = new AbortController();
  prompt.once('SIGINT', () => controller.abort());
  const cancellation = signal ? AbortSignal.any([signal, controller.signal]) : controller.signal;
  try { return await prompt.question(question, { signal: cancellation }); } finally { prompt.close(); }
} : undefined;

runFrm(process.argv.slice(2), {
  stdout: (text) => process.stdout.write(text),
  stderr: (text) => process.stderr.write(text),
  ...(ask ? { ask } : {}),
}, { program: [process.execPath, process.argv[1]] }).then((code) => {
  process.exitCode = code;
}).catch((error) => {
  const diagnostic = commandFailure(error, json);
  process.stderr.write(`${diagnostic ?? (json
    ? JSON.stringify({ error: 'command_failed', next: 'Run frm help for usage.' })
    : 'frm failed to run the requested command\nNext: Run frm help for usage.')}\n`);
  process.exitCode = 1;
});
