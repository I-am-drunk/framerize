import { loadConfig, type ServerConfig } from './config.js';
import { createLogger, type Logger, type LogOutput } from './logger.js';
import { createServer, type ServerDependencies, type ServerProvider } from './server.js';
import type { FramerizeInstance } from '../dist/core.js';

export interface ParsedArgs {
  readonly command: 'serve' | 'check';
  readonly values: Readonly<Record<string, string>>;
}

export interface CliProcess {
  readonly stdout: LogOutput;
  readonly stderr: LogOutput;
  on(event: 'SIGINT' | 'SIGTERM', listener: () => void): unknown;
  off(event: 'SIGINT' | 'SIGTERM', listener: () => void): unknown;
}

type CliInstance = Pick<FramerizeInstance, 'listen' | 'close'>;
export interface CliDependencies {
  readonly process?: CliProcess;
  readonly env?: Readonly<Record<string, string | undefined>>;
  readonly logger?: Pick<Logger, 'info'>;
  readonly createProvider?: (config: ServerConfig) => ServerProvider;
  readonly createServer?: (config: ServerConfig, dependencies: ServerDependencies) =>
    CliInstance | Promise<CliInstance>;
}

const HELP = `Framerize server
Usage: framerize-server [serve|check] [options]
  --upstream URL                  Authorized inference API base URL, env FRAMERIZE_SERVER_UPSTREAM
  --authorization-env NAME        Upstream credential environment variable
  --authorization-file FILE       Private upstream credential file
  --server-token-env NAME         Server token variable, default FRAMERIZE_SERVER_TOKEN
  --server-token-file FILE        Private server token file
  --accounts-file FILE            JSON account metadata without credentials
  --host ADDRESS                  Listen address, env FRAMERIZE_SERVER_HOST, default 127.0.0.1
  --port NUMBER                   Listen port, env FRAMERIZE_SERVER_PORT, default 8789
  --headers-timeout-ms NUMBER     Provider header wait, default 15000
  --request-headers-timeout-ms N   Incoming HTTP header wait, default 15000
  --idle-timeout-ms NUMBER        Body inactivity limit, default 60000
  --shutdown-timeout-ms NUMBER    Provider shutdown limit, default 5000
  --max-body-bytes NUMBER         JSON request limit, default 16777216
  --log-level LEVEL               debug, info, warn, error, silent
Command-line options override these environment values. Credentials use the references above.
`;
const FLAGS = new Set(['upstream', 'authorization-env', 'authorization-file', 'server-token-env',
  'server-token-file', 'accounts-file', 'host', 'port', 'headers-timeout-ms',
  'request-headers-timeout-ms', 'idle-timeout-ms', 'shutdown-timeout-ms', 'max-body-bytes', 'log-level']);

export function parseArgs(argv: readonly string[]): ParsedArgs {
  const args = [...argv];
  const command = args[0]?.startsWith('-') ? 'serve' : args.shift() ?? 'serve';
  if (command !== 'serve' && command !== 'check') throw new Error('Unknown command');
  const values: Record<string, string> = {};
  while (args.length) {
    const option = args.shift()!;
    if (!option.startsWith('--')) throw new Error('Invalid option');
    const flag = option.slice(2);
    const value = args.shift();
    if (!FLAGS.has(flag) || !value || value.startsWith('--')) throw new Error('Invalid option');
    values[flag.replace(/-([a-z])/g, (match: string, letter: string) => letter.toUpperCase())] = value;
  }
  return { command, values };
}

export async function run(argv: readonly string[] = process.argv.slice(2),
  dependencies: CliDependencies = {}): Promise<number> {
  const events = dependencies.process ?? process;
  if (argv.includes('--help') || argv.includes('-h')) { events.stdout.write(HELP); return 0; }
  const { command, values } = parseArgs(argv);
  const config = loadConfig(values, dependencies.env ?? process.env);
  const logger = dependencies.logger ?? createLogger({ stream: events.stderr, level: config.logLevel });
  const makeProvider = dependencies.createProvider ?? (await import('./provider.js')).createProvider;
  const provider = config.upstream ? makeProvider(config) : undefined;
  let app: CliInstance;
  try { app = await (dependencies.createServer ?? createServer)(config, { provider, logger }); }
  catch (error) { await provider?.close?.(); throw error; }
  if (command === 'check') {
    await app.close();
    logger.info('config.valid');
    return 0;
  }
  let resolveStopped!: () => void;
  let rejectStopped!: (reason: unknown) => void;
  let stopping: Promise<void> | undefined;
  const stopped = new Promise<void>((resolve, reject) => { resolveStopped = resolve; rejectStopped = reject; });
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
