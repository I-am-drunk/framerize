import { access } from 'node:fs/promises';
import { run, serve, type CliIo } from './cli.js';
import { ClientCleanupError, RemoteDiagnosticError, ServiceCommandCleanupError, SetupRollbackError } from './diagnostics.js';
import { FRM_HELP } from './present.js';
import { profilePath } from './profilePaths.js';
import { offerClientSetup, runClientCommand } from './clientCli.js';
import { parseArgs } from './cliArgs.js';
import { formatUp, setUp, type UpResult } from './cmdUp.js';
import { isPairingLink } from './pairingLink.js';
import { runService, serviceContext } from './serviceCli.js';

const UP_SPEC = { json: 'boolean' } as const;

export interface FrmOptions {
  /** The foreground command a service runs, such as `[node, bin/frm.js]`. */
  readonly program: readonly string[];
}

async function runUp(input: string | undefined, io: CliIo, env: NodeJS.ProcessEnv,
  program: readonly string[], json = false): Promise<{ result: UpResult; offerCancelled: boolean }> {
  const configPath = profilePath(undefined, env);
  const controller = new AbortController();
  const stop = (): void => controller.abort();
  process.on('SIGINT', stop);
  process.on('SIGTERM', stop);
  try {
    const result = await setUp(input, io, { configPath, env, signal: controller.signal,
      service: serviceContext([...program, 'serve'], env) });
    if (controller.signal.aborted) throw new RemoteDiagnosticError('aborted');
    io.stdout(json ? `${JSON.stringify(result)}\n` : formatUp(result));
    const offerCancelled = !json
      && (await offerClientSetup(configPath, io, env, undefined, controller.signal)) === 'cancelled';
    return { result, offerCancelled };
  } catch (error) {
    if (error instanceof ClientCleanupError || error instanceof ServiceCommandCleanupError) throw error;
    if (!(error instanceof SetupRollbackError) && (controller.signal.aborted
      || (error instanceof Error && error.name === 'AbortError'))) throw new RemoteDiagnosticError('aborted');
    throw error;
  } finally {
    process.off('SIGINT', stop);
    process.off('SIGTERM', stop);
  }
}

/**
 * Bare `frm`: serve a saved profile or FRM_REMOTE. On first run in a terminal, run `frm up`,
 * which asks for a pairing link or address and sets up the service, as bare `t3` leads to `t3 up`.
 */
async function start(io: CliIo, env: NodeJS.ProcessEnv, program: readonly string[]): Promise<number> {
  const configPath = profilePath(undefined, env);
  const saved = await access(configPath).then(() => true, () => false);
  if (saved || env.FRM_REMOTE) return serve(configPath, io, false);
  if (!io.ask) throw new RemoteDiagnosticError('remote_required');
  const { result, offerCancelled } = await runUp(undefined, io, env, program);
  return result.service === 'unavailable' && !offerCancelled ? serve(configPath, io, false) : 0;
}

export async function runFrm(argv: readonly string[], io: CliIo, options: FrmOptions): Promise<number> {
  const env = process.env;
  const [command, ...rest] = argv;
  if (command === undefined) return start(io, env, options.program);
  if (command === 'help' || command === '--help' || command === '-h') { io.stdout(FRM_HELP); return 0; }
  if (command === 'client') return runClientCommand(rest, io, env);
  if (command === 'service') return runService(rest, io, { program: [...options.program, 'serve'], env });
  // T3 Code's connect alias delegates to its up flow (cli/up.ts at 5a906583).
  const connect = command === 'up' || command === 'connect';
  if (connect || (isPairingLink(command) && rest.length === 0)) {
    const target = connect ? rest : [command];
    const { options: flags } = parseArgs(target.filter((value) => value.startsWith('--')), UP_SPEC);
    const links = target.filter((value) => !value.startsWith('--'));
    if (links.length > 1 || links[0]?.startsWith('-')) {
      throw new RemoteDiagnosticError('command_arguments');
    }
    const link = links[0];
    await runUp(link, io, env, options.program, flags.json === true);
    return 0;
  }
  return run(argv, io);
}
