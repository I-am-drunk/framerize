import { access } from 'node:fs/promises';
import { run, serve, type CliIo } from './cli.js';
import { RemoteDiagnosticError } from './diagnostics.js';
import { FRM_HELP } from './present.js';
import { profilePath } from './profilePaths.js';
import { offerClientSetup, runClientCommand } from './clientCli.js';
import { parseArgs } from './cliArgs.js';
import { formatUp, setUp } from './cmdUp.js';
import { isPairingLink } from './pairingLink.js';
import { runService, serviceContext } from './serviceCli.js';

const UP_SPEC = { json: 'boolean' } as const;

export interface FrmOptions {
  /** The foreground command a service runs, such as `[node, bin/frm.js]`. */
  readonly program: readonly string[];
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
  const result = await setUp(undefined, io, { configPath, env, service: serviceContext([...program, 'serve'], env) });
  io.stdout(formatUp(result));
  await offerClientSetup(configPath, io, env);
  return result.service === 'unavailable' ? serve(configPath, io, false) : 0;
}

export async function runFrm(argv: readonly string[], io: CliIo, options: FrmOptions): Promise<number> {
  const env = process.env;
  const [command, ...rest] = argv;
  if (command === undefined) return start(io, env, options.program);
  if (command === 'help' || command === '--help' || command === '-h') { io.stdout(FRM_HELP); return 0; }
  if (command === 'client') return runClientCommand(rest, io, env);
  if (command === 'service') return runService(rest, io, { program: [...options.program, 'serve'], env });
  if (command === 'up' || (isPairingLink(command) && rest.length === 0)) {
    const target = command === 'up' ? rest : [command];
    const { options: flags } = parseArgs(target.filter((value) => value.startsWith('--')), UP_SPEC);
    const links = target.filter((value) => !value.startsWith('--'));
    if (links.length > 1 || links[0]?.startsWith('-')) {
      throw new RemoteDiagnosticError('command_arguments');
    }
    const link = links[0];
    const result = await setUp(link, io, { configPath: profilePath(undefined, env), env,
      service: serviceContext([...options.program, 'serve'], env) });
    io.stdout(flags.json === true ? `${JSON.stringify(result)}\n` : formatUp(result));
    if (!flags.json) await offerClientSetup(profilePath(undefined, env), io, env);
    return 0;
  }
  return run(argv, io);
}
