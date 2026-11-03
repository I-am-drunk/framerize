import { parseConfig } from './config.js';
import { createLogger } from './logger.js';
import { createServer } from './server.js';

const HELP = `Framer Next\n\nUsage:\n  framer-next serve [options]\n  framer-next check [options]\n\nOptions:\n  --upstream URL          Upstream inference endpoint\n  --accounts FILE        JSON account metadata\n  --host ADDRESS         Listen address, default 127.0.0.1\n  --port NUMBER          Listen port, default 8788\n  --log-level LEVEL      debug, info, warn, or error\n`;

export function parseArgs(argv, env = process.env) {
  const [command = 'serve', ...args] = argv;
  const values = {
    upstream: env.FRAMER_UPSTREAM || '',
    accounts: env.FRAMER_ACCOUNTS || '',
    ...(env.FRAMER_HOST ? { host: env.FRAMER_HOST } : {}),
    ...(env.FRAMER_PORT ? { port: env.FRAMER_PORT } : {}),
  };
  for (let index = 0; index < args.length; index += 1) {
    const name = args[index];
    if (!['--upstream', '--accounts', '--host', '--port', '--log-level'].includes(name)) {
      throw new Error(`unknown option: ${name}`);
    }
    const value = args[++index];
    if (!value) throw new Error(`${name} needs a value`);
    values[name.slice(2).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())] = value;
  }
  return { command, values };
}

export async function run(argv = process.argv.slice(2)) {
  if (argv.includes('--help') || argv.includes('-h')) {
    process.stdout.write(HELP);
    return 0;
  }
  const { command, values } = parseArgs(argv);
  const config = parseConfig(values);
  const logger = createLogger({ level: values.logLevel });
  if (!['serve', 'check'].includes(command)) throw new Error(`unknown command: ${command}`);
  const app = await createServer(config, { logger });
  if (command === 'check') {
    logger.info('config.valid', { accounts: config.accounts || undefined });
    await app.close();
    return 0;
  }
  let shutdownPromise;
  let resolveClosed;
  const closed = new Promise((resolve) => { resolveClosed = resolve; });
  app.server.once('close', () => resolveClosed(0));
  const shutdown = async () => {
    shutdownPromise ??= (async () => {
      try {
        await app.close();
        logger.info('server.closed');
      } catch (error) {
        logger.error('server.close_failed', { error });
        process.exitCode = 1;
      }
    })();
    return shutdownPromise;
  };
  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);
  const address = await app.listen();
  logger.info('server.listening', { host: config.host, port: address.port });
  return closed;
}
