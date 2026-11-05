import type { PersistedConfig } from './config.js';
import { RemoteDiagnosticError } from './diagnostics.js';

/** Token variables hold values. Saved profiles only ever name them. */
export const FRM_SERVER_TOKEN = 'FRM_SERVER_TOKEN';
export const FRM_LOCAL_TOKEN = 'FRM_LOCAL_TOKEN';

type Settings = { -readonly [Key in keyof PersistedConfig]?: PersistedConfig[Key] };

/** Run settings from FRM_* variables. Flags override them; they never change a saved profile. */
export function frmSettings(env: NodeJS.ProcessEnv): Settings {
  const settings: Settings = {};
  if (env.FRM_REMOTE) settings.remote = env.FRM_REMOTE;
  if (env.FRM_HOST) settings.host = env.FRM_HOST;
  if (env.FRM_PORT !== undefined) {
    if (!/^\d{1,5}$/.test(env.FRM_PORT) || Number(env.FRM_PORT) > 65535) {
      throw new RemoteDiagnosticError('listen_invalid');
    }
    settings.port = Number(env.FRM_PORT);
  }
  if (env[FRM_SERVER_TOKEN] !== undefined) settings.serverToken = { kind: 'env', name: FRM_SERVER_TOKEN };
  if (env[FRM_LOCAL_TOKEN] !== undefined) settings.localToken = { kind: 'env', name: FRM_LOCAL_TOKEN };
  return settings;
}
