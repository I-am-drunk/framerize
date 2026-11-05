import { loadConfig, readServerToken, type ServerConfig } from './config.js';
import { createLogger, type Logger, type LogOutput } from './logger.js';
import { createServer, type ServerDependencies, type ServerProvider } from './server.js';
import { CliUsageError, ServerListenError } from './errors.js';
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
  readonly createProvider?: (config: ServerConfig, context: { readonly signal: AbortSignal;
    readonly env: Readonly<Record<string, string | undefined>> }) =>
    ServerProvider | Promise<ServerProvider>;
  readonly createServer?: (config: ServerConfig, dependencies: ServerDependencies) =>
    CliInstance | Promise<CliInstance>;
}

const HELP = `Server
Usage: framerize-server [serve|check] [options]
  --backend http|framer          Provider backend, default http
  --account-id NAME              Enabled JSON account to use with the framer backend
  --framer-origin URL            Framer API origin, default https://api.framer.com
  --upstream URL                  Authorized inference API base URL, env FRAMERIZE_SERVER_UPSTREAM
  --authorization-env NAME        Upstream credential environment variable
  --authorization-file FILE       Private upstream credential file
  --server-token-env NAME         Optional API token, default FRAMERIZE_SERVER_TOKEN
  --server-token-file FILE        Optional API token file
  --accounts-file FILE            JSON accounts with sessions or environment/file references
  --host ADDRESS                  Listen address, env FRAMERIZE_SERVER_HOST, default 127.0.0.1
  --port NUMBER                   Listen port, env FRAMERIZE_SERVER_PORT, default 8789
  --headers-timeout-ms NUMBER     Readiness, preparation and connection wait, default 15000
  --inference-timeout-ms NUMBER   Inference response wait, default 300000
  --request-headers-timeout-ms N   Incoming HTTP header wait, default 15000
  --idle-timeout-ms NUMBER        Body inactivity limit, default 60000
  --shutdown-timeout-ms NUMBER    Provider shutdown limit, default 5000
  --max-body-bytes NUMBER         JSON request limit, default 16777216
  --log-level LEVEL               debug, info, warn, error, silent
Command-line options override environment values. Without an API token, access is open.
`;
const FLAGS = new Set(['backend', 'account-id', 'framer-origin', 'upstream', 'authorization-env', 'authorization-file', 'server-token-env',
  'server-token-file', 'accounts-file', 'host', 'port', 'headers-timeout-ms', 'inference-timeout-ms',
  'request-headers-timeout-ms', 'idle-timeout-ms', 'shutdown-timeout-ms', 'max-body-bytes', 'log-level']);

export function parseArgs(argv: readonly string[]): ParsedArgs {
  const args = [...argv];
  const command = args[0]?.startsWith('-') ? 'serve' : args.shift() ?? 'serve';
  if (command !== 'serve' && command !== 'check') throw new CliUsageError('Unknown command');
  const values: Record<string, string> = {};
  while (args.length) {
    const option = args.shift()!;
    if (!option.startsWith('--')) throw new CliUsageError('Invalid option');
    const flag = option.slice(2);
    const value = args.shift();
    if (!FLAGS.has(flag) || !value || value.startsWith('--')) throw new CliUsageError('Invalid option');
    values[flag.replace(/-([a-z])/g, (match: string, letter: string) => letter.toUpperCase())] = value;
  }
  return { command, values };
}

export async function run(argv: readonly string[] = process.argv.slice(2),
  dependencies: CliDependencies = {}): Promise<number> {
  const events = dependencies.process ?? process;
  if (argv.includes('--help') || argv.includes('-h')) { events.stdout.write(HELP); return 0; }
  const { command, values } = parseArgs(argv);
  const env = dependencies.env ?? process.env;
  const config = loadConfig(values, env);
  const logger = dependencies.logger ?? createLogger({ stream: events.stderr, level: config.logLevel });
  const controller = new AbortController();
  let provider: ServerProvider | undefined;
  let app: CliInstance | undefined;
  let closing: Promise<void> | undefined;
  let resolveStopped!: () => void;
  const stopped = new Promise<void>(resolve => { resolveStopped = resolve; });
  const close = () => {
    closing ??= Promise.resolve().then(() => app ? app.close() : provider?.close?.());
    closing.catch(() => {});
    return closing;
  };
  const stop = () => {
    controller.abort();
    resolveStopped();
    if (app) void close();
  };
  events.on('SIGINT', stop);
  events.on('SIGTERM', stop);
  try {
    const serverConfig = loadConfig({
      ...config, serverTokenFile: '', serverToken: await readServerToken(config),
    }, env);
    const makeProvider: NonNullable<CliDependencies['createProvider']> =
      dependencies.createProvider ?? (await import('./backend.js')).createConfiguredProvider;
    if (controller.signal.aborted) return 0;
    provider = config.upstream || config.backend === 'framer'
      ? await makeProvider(config, { signal: controller.signal, env }) : undefined;
    if (controller.signal.aborted) return 0;
    app = await (dependencies.createServer ?? createServer)(serverConfig, { provider, logger });
    if (controller.signal.aborted) return 0;
    if (command === 'check') {
      await close();
      if (!controller.signal.aborted) logger.info('config.valid');
      return 0;
    }
    let address: Awaited<ReturnType<CliInstance['listen']>>;
    try { address = await app.listen(); }
    catch { throw new ServerListenError(); }
    if (!controller.signal.aborted) logger.info('server.listening', { port: address.port });
    await stopped;
    return 0;
  } catch (error) {
    if (!controller.signal.aborted) throw error;
    return 0;
  } finally {
    try { await close(); }
    finally {
      events.off('SIGINT', stop);
      events.off('SIGTERM', stop);
    }
  }
}
