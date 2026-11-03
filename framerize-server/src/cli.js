import { loadConfig } from './config.js';
import { createLogger } from './logger.js';
import { createServer } from './server.js';

const HELP = `Framerize server
Usage: framerize-server [serve|check] [options]
  --upstream URL                  Authorized inference API base URL
  --authorization-env NAME        Upstream credential environment variable
  --authorization-file FILE       Private upstream credential file
  --server-token-env NAME         Server token variable, default FRAMERIZE_SERVER_TOKEN
  --server-token-file FILE        Private server token file
  --accounts-file FILE            JSON account metadata without credentials
  --host ADDRESS                  Listen address, default 127.0.0.1
  --port NUMBER                   Listen port, default 8789
  --headers-timeout-ms NUMBER     Provider header wait, default 15000
  --request-headers-timeout-ms N   Incoming HTTP header wait, default 15000
  --idle-timeout-ms NUMBER        Body inactivity limit, default 60000
  --shutdown-timeout-ms NUMBER    Provider shutdown limit, default 5000
  --max-body-bytes NUMBER         JSON request limit, default 16777216
  --log-level LEVEL               debug, info, warn, error, silent
`;
const FLAGS = new Set(['upstream', 'authorization-env', 'authorization-file', 'server-token-env',
  'server-token-file', 'accounts-file', 'host', 'port', 'headers-timeout-ms',
  'request-headers-timeout-ms', 'idle-timeout-ms', 'shutdown-timeout-ms', 'max-body-bytes', 'log-level']);

export function parseArgs(argv) {
  const args = [...argv];
  const command = args[0]?.startsWith('-') ? 'serve' : args.shift() ?? 'serve';
  if (!['serve', 'check'].includes(command)) throw new Error('Unknown command');
  const values = {};
  while (args.length) {
    const option = args.shift();
    if (!option.startsWith('--')) throw new Error('Invalid option');
    const flag = option.slice(2);
    const value = args.shift();
    if (!FLAGS.has(flag) || !value || value.startsWith('--')) throw new Error('Invalid option');
    values[flag.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())] = value;
  }
  return { command, values };
}

export async function run(argv = process.argv.slice(2), dependencies = {}) {
  const events = dependencies.process ?? process;
  if (argv.includes('--help') || argv.includes('-h')) { events.stdout.write(HELP); return 0; }
  const { command, values } = parseArgs(argv);
  const config = loadConfig(values, dependencies.env ?? process.env);
  const logger = dependencies.logger ?? createLogger({ stream: events.stderr, level: config.logLevel });
  const makeProvider = dependencies.createProvider ?? (await import('./provider.js')).createProvider;
  const provider = config.upstream ? makeProvider(config) : undefined;
  let app;
  try { app = await (dependencies.createServer ?? createServer)(config, { provider, logger }); }
  catch (error) { await provider?.close?.(); throw error; }
  if (command === 'check') {
    await app.close();
    logger.info('config.valid');
    return 0;
  }
  let resolveStopped;
  let rejectStopped;
  let stopping;
  const stopped = new Promise((resolve, reject) => { resolveStopped = resolve; rejectStopped = reject; });
  stopped.catch(() => {});
  const stop = () => {
    stopping ??= app.close().then(resolveStopped, rejectStopped);
  };
  events.on('SIGINT', stop);
  events.on('SIGTERM', stop);
  try {
    const address = await app.listen();
    logger.info('server.listening', { port: address.port });
    await stopped;
    return 0;
  } finally {
    events.off('SIGINT', stop);
    events.off('SIGTERM', stop);
    await app.close();
  }
}
