const { pathToFileURL } = require('node:url');
const path = require('node:path');
const { sanitizeError } = require('../src/logger.js');

async function main() {
  const { run } = await import(pathToFileURL(path.join(__dirname, '../src/cli.js')));
  process.exitCode = await run();
}

main().catch((error) => {
  process.stderr.write(`${JSON.stringify({ event: 'fatal', message: sanitizeError(error) })}\n`);
  process.exitCode = 1;
});
