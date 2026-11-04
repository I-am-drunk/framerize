import { loadConfigFile, resolveConfig } from './configStore.js';
import { loadCredential } from './credentials.js';
import { createRemoteBackend } from './remoteBackend.js';
import { DEFAULT_BACKEND_TIMINGS } from './backendConfig.js';
import type { RemoteConfig } from './config.js';
import { RemoteDiagnosticError } from './diagnostics.js';
import { applyListenOverrides, type ListenOverrides } from './listenOptions.js';

export interface Resolved {
  readonly config: RemoteConfig;
  readonly localToken: string;
  readonly backend: ReturnType<typeof createRemoteBackend>;
}

export async function resolveInstance(
  configPath: string,
  env: NodeJS.ProcessEnv = process.env,
  overrides?: ListenOverrides,
): Promise<Resolved> {
  let config: RemoteConfig;
  let endpoint: ReturnType<typeof resolveConfig>;
  try {
    config = await loadConfigFile(configPath);
    if (overrides) config = applyListenOverrides(config, overrides);
    endpoint = resolveConfig(config);
  }
  catch (error) {
    if (error instanceof RemoteDiagnosticError) throw error;
    throw new RemoteDiagnosticError('profile_invalid', error instanceof Error ? error.message : undefined);
  }
  const [serverToken, localToken] = await Promise.all([
    loadCredential(config.serverToken, env).catch(() => {
      throw new RemoteDiagnosticError('server_credential', 'credential is unavailable or invalid');
    }),
    loadCredential(config.localToken, env).catch(() => {
      throw new RemoteDiagnosticError('local_credential', 'credential is unavailable or invalid');
    }),
  ]);
  const backend = createRemoteBackend({
    endpoint, serverToken, ...DEFAULT_BACKEND_TIMINGS,
    inferenceTimeoutMs: config.inferenceTimeoutMs,
  });
  return { config, localToken, backend };
}
