import { parseArgs } from './cliArgs.js';
import { runSetup, type Ask } from './cmdSetup.js';
import { runCheck, startServe, StartupCleanupError, StartupReadinessError, type ServeHandle } from './cmdRun.js';
import { HELP, SETUP_HELP, SERVE_HELP, VERSION } from './present.js';
import { profilePath } from './profilePaths.js';
import { ClientCleanupError, diagnostic, diagnosticText, RemoteDiagnosticError } from './diagnostics.js';
import { LISTEN_SPEC, parseListenOptions, type ListenOverrides } from './listenOptions.js';
import { isLoopbackHost } from './hostPolicy.js';
import { offerClientSetup } from './clientCli.js';
import { withEnvOverrides } from './frmEnv.js';

const GLOBAL_SPEC = { config: 'string', json: 'boolean' } as const;

export interface CliIo {
  readonly stdout: (text: string) => void;
  readonly stderr: (text: string) => void;
  /** Asks one question when stdin and stdout are a terminal. */
  readonly ask?: Ask;
}

function splitGlobals(rest: readonly string[]): { configPath: string | undefined; json: boolean; passthrough: string[] } {
  const passthrough: string[] = [];
  const globals: string[] = [];
  for (let index = 0; index < rest.length; index += 1) {
    const token = rest[index]!;
    if (token !== '--config' && token !== '--json') { passthrough.push(token); continue; }
    globals.push(token);
    if (token === '--config') globals.push(rest[++index] ?? '');
  }
  const { options } = parseArgs(globals, GLOBAL_SPEC);
  return {
    configPath: typeof options.config === 'string' ? options.config : undefined,
    json: options.json === true,
    passthrough,
  };
}

export async function run(argv: readonly string[], io: CliIo): Promise<number> {
  const [command, ...rest] = argv;
  if (command === undefined || command === 'help' || command === '--help' || command === '-h') {
    io.stdout(HELP);
    return 0;
  }
  if (command === 'version' || command === '--version' || command === '-v') {
    io.stdout(`${VERSION}\n`);
    return 0;
  }
  let globals: ReturnType<typeof splitGlobals>;
  try { globals = splitGlobals(rest); }
  catch (error) {
    if (command === 'setup') throw new RemoteDiagnosticError('setup_arguments');
    throw error;
  }
  const { configPath: explicit, json, passthrough } = globals;
  if ((command === 'setup' || command === 'serve') && passthrough.length === 1
    && ['--help', '-h'].includes(passthrough[0]!)) {
    io.stdout(command === 'setup' ? SETUP_HELP : SERVE_HELP);
    return 0;
  }
  const configPath = profilePath(explicit);
  if (command === 'setup') {
    const controller = new AbortController();
    const stop = (): void => controller.abort();
    process.on('SIGINT', stop);
    process.on('SIGTERM', stop);
    try {
      const overrides = await runSetup(passthrough, configPath, process.env, io.ask, controller.signal);
      controller.signal.throwIfAborted();
      io.stdout(json ? `${JSON.stringify({ saved: true })}\n` : 'Profile saved.\n');
      if (passthrough.includes('--start')) return serve(configPath, io, json, overrides, true);
      if (!json) {
        const clientEnv = withEnvOverrides(process.env, {
          ...(overrides.host !== undefined ? { FRM_HOST: undefined } : {}),
          ...(overrides.port !== undefined ? { FRM_PORT: undefined } : {}),
        });
        const offer = await offerClientSetup(configPath, io, clientEnv, undefined, controller.signal);
        if (offer === 'cancelled' || controller.signal.aborted) throw new RemoteDiagnosticError('aborted');
        io.stdout('Next: frm serve\n');
      }
      return 0;
    } finally {
      process.off('SIGINT', stop);
      process.off('SIGTERM', stop);
    }
  }
  if (command === 'check') {
    const { options } = parseArgs(passthrough, { tailscale: 'boolean' });
    const controller = new AbortController();
    const stop = (): void => controller.abort();
    process.on('SIGINT', stop);
    process.on('SIGTERM', stop);
    try {
      const result = await runCheck(configPath, json, process.env, options.tailscale === true, controller.signal);
      io.stdout(`${result.text}\n`);
      return result.exitCode;
    } finally {
      process.off('SIGINT', stop);
      process.off('SIGTERM', stop);
    }
  }
  if (command === 'serve') {
    const { options } = parseArgs(passthrough, LISTEN_SPEC);
    return serve(configPath, io, json, parseListenOptions(options));
  }
  io.stderr(json ? '{"error":"unknown_command","next":"Run frm help."}\n'
    : 'unknown command; run `frm help`\n');
  return 2;
}

export async function serve(configPath: string, io: CliIo, json: boolean,
  overrides: ListenOverrides = {}, setupClients = false): Promise<number> {
  const controller = new AbortController();
  let handle: ServeHandle | undefined;
  let stoppedCleanly = false;
  let resolveStopped!: () => void;
  const stopped = new Promise<void>(resolve => { resolveStopped = resolve; });
  const stop = (): void => { controller.abort(); resolveStopped(); };
  process.on('SIGINT', stop);
  process.on('SIGTERM', stop);
  try {
    handle = await startServe(configPath, process.env, controller.signal, overrides);
    const { host, port } = handle.address;
    const address = host === '0.0.0.0' ? '127.0.0.1' : host === '::' ? '[::1]'
      : host.includes(':') && !host.startsWith('[') ? `[${host}]` : host;
    const exposed = !isLoopbackHost(host);
    const warning = exposed ? `Non-loopback HTTP enabled on port ${port}.\n` : '';
    io.stdout(json ? `${JSON.stringify({ ready: true, listening: { host, port },
      ...(exposed ? { exposure: 'non-loopback-http' } : {}) })}\n`
      : `Remote readiness passed.\n${warning}Local API: http://${address}:${port}/v1\nStop: Ctrl+C\n`);
    if (setupClients && !json) {
      const offer = await offerClientSetup(configPath, io, withEnvOverrides(process.env,
        { FRM_HOST: host, FRM_PORT: String(port) }),
        undefined, controller.signal);
      if (offer === 'cancelled') stop();
    }
    await stopped;
    stoppedCleanly = true;
    return 0;
  } catch (error) {
    if (error instanceof ClientCleanupError) throw error;
    if (controller.signal.aborted && !(error instanceof StartupCleanupError)) {
      stoppedCleanly = true;
      return 0;
    }
    if (!(error instanceof StartupReadinessError)) throw error;
    const result = { ready: false, reason: error.reason, diagnostic: diagnostic(error.code) } as const;
    io.stdout(`${json ? JSON.stringify(result) : diagnosticText(error.code)}\n`);
    return 1;
  } finally {
    try { await handle?.close(); }
    catch { throw new RemoteDiagnosticError('cleanup_failed'); }
    finally {
      process.off('SIGINT', stop);
      process.off('SIGTERM', stop);
    }
    if (stoppedCleanly) io.stdout(json ? '{"stopped":true}\n' : 'Stopped.\n');
  }
}
