// Inert child ownership fixture. No SDK, account state or network.
import fs from 'node:fs';
import { setTimeout as sleep } from 'node:timers/promises';
const at = name => new URL(name, import.meta.url);
fs.appendFileSync(at('./starts'), '1\n', { mode: 0o600 });
fs.writeFileSync(at('./ready.tmp'), String(process.pid), { flag: 'wx', mode: 0o600 });
fs.renameSync(at('./ready.tmp'), at('./ready'));
while (!fs.existsSync(at('./release'))) await sleep(5);
await sleep(220);
await import('./operation.mjs');
