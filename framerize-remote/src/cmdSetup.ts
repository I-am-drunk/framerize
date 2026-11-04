import { parseArgs } from './cliArgs.js';
import { saveConfigFile } from './configStore.js';
import { parseRemoteEndpoint } from './remoteUrl.js';
import type { CredentialReference } from './credentials.js';
import type { PersistedConfig } from './config.js';
import { RemoteDiagnosticError } from './diagnostics.js';
import { parseListenOptions } from './listenOptions.js';

const SETUP_SPEC = {
  remote: 'string', 'server-token-env': 'string', 'server-token-file': 'string',
  'local-token-env': 'string', 'local-token-file': 'string', host: 'string',
  port: 'string', 'allow-insecure-remote': 'boolean', 'allow-non-loopback': 'boolean',
  start: 'boolean',
  'inference-timeout-ms': 'string',
} as const;

function reference(env: unknown, file: unknown, role: 'server' | 'local'): CredentialReference {
  const code = role === 'server' ? 'server_reference' : 'local_reference';
  if (typeof env === 'string' && typeof file === 'string') throw new RemoteDiagnosticError(code);
  if (typeof env === 'string') return { kind: 'env', name: env };
  if (typeof file === 'string') return { kind: 'file', path: file };
  throw new RemoteDiagnosticError(code);
}

export async function runSetup(argv: readonly string[], configPath: string): Promise<string> {
  let parsed: ReturnType<typeof parseArgs>;
  try { parsed = parseArgs(argv, SETUP_SPEC); }
  catch { throw new RemoteDiagnosticError('setup_arguments'); }
  const { options } = parsed;
  const listen = parseListenOptions(options);
  const inferenceTimeout = options['inference-timeout-ms'];
  if (typeof inferenceTimeout === 'string' && !/^\d+$/.test(inferenceTimeout)) {
    throw new RemoteDiagnosticError('inference_invalid', 'invalid inference timeout');
  }
  if (typeof options.remote !== 'string') throw new RemoteDiagnosticError('remote_required');
  const insecure = options['allow-insecure-remote'] === true;
  try { parseRemoteEndpoint(options.remote, insecure); }
  catch { throw new RemoteDiagnosticError('remote_invalid'); }
  const config: PersistedConfig = {
    remote: options.remote,
    serverToken: reference(options['server-token-env'], options['server-token-file'], 'server'),
    localToken: reference(options['local-token-env'], options['local-token-file'], 'local'),
    ...listen,
    ...(typeof inferenceTimeout === 'string' ? { inferenceTimeoutMs: Number(inferenceTimeout) } : {}),
    allowInsecureRemote: insecure,
    allowNonLoopback: options['allow-non-loopback'] === true,
  };
  await saveConfigFile(configPath, config);
  return 'Profile saved.';
}
