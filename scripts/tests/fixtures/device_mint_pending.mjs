// Deterministic cancellation fixture. No SDK, account files or network.
import fs from 'node:fs';
import { readInput, validateInput, verifyDescriptors } from './protocol.mjs';

verifyDescriptors();
const input = readInput();
validateInput(input);
if (input.version !== 2 || input.device.id !== 'device-bridge-fixture'
    || input.target.project_id !== 'hang') throw new Error('fixture input');
fs.writeFileSync(new URL('./ready.tmp', import.meta.url), String(process.pid),
  { flag: 'wx', mode: 0o600 });
fs.renameSync(new URL('./ready.tmp', import.meta.url), new URL('./ready', import.meta.url));
await new Promise(() => { setInterval(() => {}, 1000); });
