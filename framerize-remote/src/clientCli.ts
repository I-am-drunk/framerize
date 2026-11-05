import { homedir } from 'node:os';
import { dirname } from 'node:path';
import type { CliIo } from './cli.js';
import { loadRunConfig } from './configStore.js';
import { loadCredential } from './credentials.js';
import { RemoteDiagnosticError } from './diagnostics.js';
import { profilePath } from './profilePaths.js';
import { CLIENT_HELP, nodeRunner, offerClients, parseClientOptions, runClient,
  type ClientSetupDeps, type RemoteTarget } from './compatibility/index.js';

export async function clientDependencies(configPath: string, io: CliIo,
  env: NodeJS.ProcessEnv, target?: RemoteTarget): Promise<ClientSetupDeps> {
  if (!target) {
    const config = await loadRunConfig(configPath, env);
    if (config.port === 0) throw new RemoteDiagnosticError('listen_invalid',
      'Set FRM_PORT to the running Remote port before starting a client.');
    const host = config.host === '0.0.0.0' ? '127.0.0.1' : config.host === '::' ? '[::1]'
      : config.host.includes(':') && !config.host.startsWith('[') ? `[${config.host}]` : config.host;
    let localToken: string | undefined;
    if (config.localToken) {
      try { localToken = await loadCredential(config.localToken, env); }
      catch { throw new RemoteDiagnosticError('local_credential'); }
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
  const path = profilePath(undefined, env);
  const target = options.undo ? { origin: 'http://127.0.0.1:8788' } : undefined;
  return runClient(argv, await clientDependencies(path, io, env, target));
}

export async function offerClientSetup(configPath: string, io: CliIo, env: NodeJS.ProcessEnv,
  dependencies?: ClientSetupDeps): Promise<void> {
  if (!io.ask) return;
  const answer = await io.ask('Set up Claude Code or Codex for this connection? [y/N] ');
  if (!['y', 'yes'].includes(answer?.trim().toLowerCase() ?? '')) return;
  await offerClients(dependencies ?? await clientDependencies(configPath, io, env));
  io.stdout('Start a client with frm client claude or frm client codex.\n');
}
