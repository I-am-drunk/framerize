import { createAccountBackend } from './account-backend.js';
import { createProvider } from './provider.js';
import { createServerMinter } from './castle-loader.js';
import type { ServerConfig } from './config.js';
import type { ServerProvider } from './server.js';

export async function createConfiguredProvider(config: ServerConfig,
  context: { readonly signal: AbortSignal; readonly env?: Readonly<Record<string, string | undefined>> }):
  Promise<ServerProvider> {
  if (config.backend === 'http') {
    const env = context.env ?? process.env;
    return createProvider({ ...config,
      ...(config.authorizationEnv ? { credential: () => env[config.authorizationEnv] } : {}),
    });
  }
  return createAccountBackend({ accountsFile: config.accountsFile,
    ...(config.accountId ? { accountId: config.accountId } : {}),
    ...(context.env ? { env: context.env } : {}), signal: context.signal,
    origin: config.framerOrigin, maxBodyBytes: config.maxBodyBytes,
    headersTimeoutMs: config.headersTimeoutMs, idleTimeoutMs: config.idleTimeoutMs,
    inferenceTimeoutMs: config.inferenceTimeoutMs,
    createMinter: ({ signal }) => createServerMinter(signal),
  });
}
