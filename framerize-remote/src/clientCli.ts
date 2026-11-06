import { homedir } from 'node:os';
import { dirname } from 'node:path';
import type { CliIo } from './cli.js';
import { loadRunConfig } from './configStore.js';
import { loadCredential } from './credentials.js';
import { ClientCleanupError, RemoteDiagnosticError } from './diagnostics.js';
import { profilePath } from './profilePaths.js';
import { CLIENT_HELP, nodeRunner, offerClients, parseClientOptions, runClient,
  type ClientSetupDeps, type RemoteTarget } from './compatibility/index.js';

export async function clientDependencies(configPath: string, io: CliIo,
  env: NodeJS.ProcessEnv, target?: RemoteTarget, afterSetup = false,
  signal?: AbortSignal): Promise<ClientSetupDeps> {
  signal?.throwIfAborted();
  if (!target) {
    const config = await loadRunConfig(configPath, env, afterSetup, signal);
    if (config.port === 0) throw new RemoteDiagnosticError('listen_invalid',
      'Set FRM_PORT to the running Remote port before starting a client.');
    const host = config.host === '0.0.0.0' ? '127.0.0.1' : config.host === '::' ? '[::1]'
      : config.host.includes(':') && !config.host.startsWith('[') ? `[${config.host}]` : config.host;
    let localToken: string | undefined;
    if (config.localToken) {
      try { localToken = await loadCredential(config.localToken, env, signal); }
      catch { signal?.throwIfAborted(); throw new RemoteDiagnosticError('local_credential'); }
    }
    target = { origin: `http://${host}:${config.port}`, ...(localToken ? { localToken } : {}) };
  }
  return { platform: process.platform, env, home: homedir(), stateDir: dirname(configPath),
    target, runner: nodeRunner(), io, ...(io.ask ? { prompt: io.ask } : {}) };
}

export async function runClientCommand(argv: readonly string[], io: CliIo,
  env: NodeJS.ProcessEnv = process.env): Promise<number> {
  if (!argv.length || ['help', '--help', '-h'].includes(argv[0]!)) {
    io.stdout(CLIENT_HELP);
    return 0;
  }
  let options: ReturnType<typeof parseClientOptions>;
  try { options = parseClientOptions(argv, env); } catch { options = null; }
  if (!options) {
    io.stderr('Invalid client options. Run frm client --help.\n');
    return 2;
  }
  const path = profilePath(options.config, env);
  const target = options.undo ? { origin: 'http://127.0.0.1:8788' } : undefined;
  const controller = new AbortController();
  const stop = (): void => controller.abort();
  process.on('SIGINT', stop);
  process.on('SIGTERM', stop);
  const detach = (): void => {
    process.off('SIGINT', stop);
    process.off('SIGTERM', stop);
  };
  try {
    const deps = await clientDependencies(path, io, env, target, false, controller.signal);
    controller.signal.throwIfAborted();
    const result = await runClient(argv, { ...deps, signal: controller.signal,
      runner: { ...deps.runner, run(command, environment, signal) {
        if (!signal) {
          controller.signal.throwIfAborted();
          // The launched client owns interactive signal handling.
          detach();
        }
        return deps.runner.run(command, environment, signal);
      } },
    });
    controller.signal.throwIfAborted();
    return result;
  } finally {
    detach();
  }
}

export async function offerClientSetup(configPath: string, io: CliIo, env: NodeJS.ProcessEnv,
  dependencies?: ClientSetupDeps, signal?: AbortSignal, afterSetup = false): Promise<'cancelled' | void> {
  const cancellation = signal ?? dependencies?.signal;
  if (cancellation?.aborted) return 'cancelled';
  if (!io.ask) return;
  try {
    const answer = await io.ask('Set up Claude Code or Codex for this connection? [y/N] ', cancellation);
    if (cancellation?.aborted) return 'cancelled';
    if (!['y', 'yes'].includes(answer?.trim().toLowerCase() ?? '')) return;
    const deps = dependencies ?? await clientDependencies(configPath, io, env, undefined, afterSetup, cancellation);
    await offerClients(cancellation ? { ...deps, signal: cancellation } : deps);
    if (cancellation?.aborted) return 'cancelled';
    let defaultPath: string | undefined;
    try { defaultPath = profilePath(undefined, env); } catch { /* An explicit profile can bypass an invalid home. */ }
    const custom = configPath !== defaultPath;
    io.stdout(`Start a client with frm client claude or frm client codex${custom
      ? ', adding --config FILE for this profile.' : '.'}\n`);
    if (custom) io.stdout(`Profile file: ${configPath}\n`);
  } catch (error) {
    if (error instanceof ClientCleanupError) throw error;
    if (cancellation?.aborted || (error instanceof Error && error.name === 'AbortError')) return 'cancelled';
    throw error;
  }
}
