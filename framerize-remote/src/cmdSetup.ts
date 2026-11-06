import { parseArgs } from './cliArgs.js';
import { saveConfigFile, validateConfigForSave } from './configStore.js';
import { withProfileLock } from './profileLock.js';
import { parseRemoteEndpoint } from './remoteUrl.js';
import type { CredentialReference } from './credentials.js';
import type { PersistedConfig } from './config.js';
import { RemoteDiagnosticError } from './diagnostics.js';
import { parseListenOptions, type ListenOverrides } from './listenOptions.js';
import { frmSettings } from './frmEnv.js';

const SETUP_SPEC = {
  remote: 'string', 'server-token-env': 'string', 'server-token-file': 'string',
  'local-token-env': 'string', 'local-token-file': 'string', host: 'string',
  port: 'string', 'allow-insecure-remote': 'boolean', 'allow-non-loopback': 'boolean',
  start: 'boolean',
  'inference-timeout-ms': 'string',
} as const;

function reference(env: unknown, file: unknown, role: 'server' | 'local'): CredentialReference | undefined {
  const code = role === 'server' ? 'server_reference' : 'local_reference';
  if (typeof env === 'string' && typeof file === 'string') throw new RemoteDiagnosticError(code);
  if (typeof env === 'string') return { kind: 'env', name: env };
  if (typeof file === 'string') return { kind: 'file', path: file };
  return undefined;
}

function optionalReference(key: 'serverToken' | 'localToken', env: unknown, file: unknown,
  role: 'server' | 'local'): Partial<Record<typeof key, CredentialReference>> {
  const value = reference(env, file, role);
  return value ? { [key]: value } : {};
}

/** Asks one question in a terminal. Absent when stdin or stdout is not a terminal. */
export type Ask = (question: string, signal?: AbortSignal) => Promise<string>;

export async function runSetup(argv: readonly string[], configPath: string,
  env: NodeJS.ProcessEnv = process.env, ask?: Ask, signal?: AbortSignal): Promise<ListenOverrides> {
  signal?.throwIfAborted();
  let parsed: ReturnType<typeof parseArgs>;
  try { parsed = parseArgs(argv, SETUP_SPEC); }
  catch { throw new RemoteDiagnosticError('setup_arguments'); }
  const { options } = parsed;
  const flags = parseListenOptions(options);
  const frm = frmSettings(flags.port === undefined ? env : { ...env, FRM_PORT: undefined });
  const listen = { ...(frm.host ? { host: frm.host } : {}), ...(frm.port !== undefined ? { port: frm.port } : {}),
    ...flags };
  const inferenceTimeout = options['inference-timeout-ms'];
  if (typeof inferenceTimeout === 'string' && !/^\d+$/.test(inferenceTimeout)) {
    throw new RemoteDiagnosticError('inference_invalid', 'invalid inference timeout');
  }
  let remote = typeof options.remote === 'string' ? options.remote : frm.remote;
  if (remote === undefined && ask) remote = (await ask('Server address, such as http://100.64.0.1:8789: ', signal)).trim();
  signal?.throwIfAborted();
  if (!remote) throw new RemoteDiagnosticError('remote_required');
  const insecure = options['allow-insecure-remote'] === true;
  try { parseRemoteEndpoint(remote, insecure); }
  catch { throw new RemoteDiagnosticError('remote_invalid'); }
  const config: PersistedConfig = {
    remote,
    ...optionalReference('serverToken', options['server-token-env'], options['server-token-file'], 'server'),
    ...optionalReference('localToken', options['local-token-env'], options['local-token-file'], 'local'),
    ...listen,
    ...(typeof inferenceTimeout === 'string' ? { inferenceTimeoutMs: Number(inferenceTimeout) } : {}),
    allowInsecureRemote: insecure,
    allowNonLoopback: options['allow-non-loopback'] === true,
  };
  validateConfigForSave(config);
  await withProfileLock(configPath, async () => {
    signal?.throwIfAborted();
    await saveConfigFile(configPath, config);
  });
  return flags;
}
