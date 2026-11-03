import { appendFileSync } from 'node:fs';
const [name, ...args] = process.argv.slice(2);
const fixture = process.env.FRAMER_NEXT_TEST_COMMAND
  ? JSON.parse(process.env.FRAMER_NEXT_TEST_COMMAND) : null;
if (fixture) {
if (name !== 'tar') appendFileSync(fixture.calls, `${args.join(' ')}\n`);
if (name === 'tar') console.log(JSON.stringify({ name: 'framerize-nexus', version: fixture.version }));
if (name === 'git' && args[0] === 'rev-parse') console.log('a'.repeat(40));
if (name === 'git' && args[0] === 'status' && fixture.dirty) console.log(' M source.js');
if (name === 'git' && args[0] === 'cat-file' && args[2] !== fixture.commit) process.exit(1);
if (name === 'sha256sum' && !fixture.checksumValid) process.exit(1);
if (name === 'npm' && args[0] === 'whoami' && !fixture.authenticated) process.exit(1);
if (name === 'npm' && args[0] === 'view' && !fixture.published) {
  console.error(fixture.registryError || 'E404');
  process.exit(1);
}
}
