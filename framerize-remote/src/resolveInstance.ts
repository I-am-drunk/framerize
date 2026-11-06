import { loadRunConfig, resolveConfig } from './configStore.js';
import { loadCredential } from './credentials.js';
import { createRemoteBackend } from './remoteBackend.js';
import { DEFAULT_BACKEND_TIMINGS } from './backendConfig.js';
import type { RemoteConfig } from './config.js';
import { RemoteDiagnosticError } from './diagnostics.js';
import { applyListenOverrides, type ListenOverrides } from './listenOptions.js';

export interface Resolved {
  readonly config: RemoteConfig;
  readonly localToken: string | null;
  readonly backend: ReturnType<typeof createRemoteBackend>;
}

export async function resolveInstance(
  configPath: string,
  env: NodeJS.ProcessEnv = process.env,
  overrides?: ListenOverrides,
  afterSetup = false,
): Promise<Resolved> {
  let config: RemoteConfig;
  let endpoint: ReturnType<typeof resolveConfig>;
  try {
    const selectedEnv: NodeJS.ProcessEnv = Object.create(env);
    if (overrides?.host !== undefined) Object.defineProperty(selectedEnv, 'FRM_HOST', { value: undefined });
    if (overrides?.port !== undefined) Object.defineProperty(selectedEnv, 'FRM_PORT', { value: undefined });
    config = await loadRunConfig(configPath, selectedEnv, afterSetup);
    if (overrides) config = applyListenOverrides(config, overrides);
    endpoint = resolveConfig(config);
  }
  catch (error) {
    if (error instanceof RemoteDiagnosticError) throw error;
    throw new RemoteDiagnosticError('profile_invalid', error instanceof Error ? error.message : undefined);
  }
  const optional = (reference: RemoteConfig['serverToken'], code: 'server_credential' | 'local_credential') =>
    reference === undefined ? Promise.resolve(null) : loadCredential(reference, env).catch(() => {
      throw new RemoteDiagnosticError(code, 'credential is unavailable or invalid');
    });
  const [serverToken, localToken] = await Promise.all([
    optional(config.serverToken, 'server_credential'), optional(config.localToken, 'local_credential'),
  ]);
  const backend = createRemoteBackend({
    endpoint, serverToken, ...DEFAULT_BACKEND_TIMINGS,
    inferenceTimeoutMs: config.inferenceTimeoutMs,
  });
  return { config, localToken, backend };
}
