import { parseArgs } from './cliArgs.js';
import { saveConfigFile } from './configStore.js';
import { parseRemoteEndpoint } from './remoteUrl.js';
import type { CredentialReference } from './credentials.js';
import type { PersistedConfig } from './config.js';

const SETUP_SPEC = {
  remote: 'string', 'server-token-env': 'string', 'server-token-file': 'string',
  'local-token-env': 'string', 'local-token-file': 'string', host: 'string',
  port: 'string', 'allow-insecure-remote': 'boolean', 'allow-non-loopback': 'boolean',
  start: 'boolean',
} as const;

function reference(env: unknown, file: unknown, role: string): CredentialReference {
  if (typeof env === 'string' && typeof file === 'string') throw new Error(`${role} token: use env or file, not both`);
  if (typeof env === 'string') return { kind: 'env', name: env };
  if (typeof file === 'string') return { kind: 'file', path: file };
  throw new Error(`${role} token reference is required`);
}

export async function runSetup(argv: readonly string[], configPath: string): Promise<string> {
  const { options } = parseArgs(argv, SETUP_SPEC);
  if (typeof options.port === 'string' && !/^\d+$/.test(options.port)) throw new Error('invalid listen port');
  if (typeof options.remote !== 'string') throw new Error('--remote is required');
  const insecure = options['allow-insecure-remote'] === true;
  parseRemoteEndpoint(options.remote, insecure);
  const config: PersistedConfig = {
    remote: options.remote,
    serverToken: reference(options['server-token-env'], options['server-token-file'], 'server'),
    localToken: reference(options['local-token-env'], options['local-token-file'], 'local'),
    ...(typeof options.host === 'string' ? { host: options.host } : {}),
    ...(typeof options.port === 'string' ? { port: Number(options.port) } : {}),
    allowInsecureRemote: insecure,
    allowNonLoopback: options['allow-non-loopback'] === true,
  };
  await saveConfigFile(configPath, config);
  return 'Profile saved.';
}
