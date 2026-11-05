import { createInstance, type BackendReadiness, type FramerizeInstance,
  type InferenceBackend, type InferenceRequest, type InferenceResponse, type RequestContext } from '../dist/core.js';
import { loadConfig, readServerToken } from './config.js';
import { loadAccounts } from './accounts.js';
import { ProviderError } from './errors.js';
import type { Logger } from './logger.js';
import { acceptsPrivateNetworkConnection } from './private-network.js';

export interface ServerProvider {
  forward?(request: InferenceRequest): InferenceResponse | Promise<InferenceResponse>;
  ready?(context: RequestContext): BackendReadiness | Promise<BackendReadiness>;
  close?(): void | Promise<void>;
}

export interface ServerDependencies {
  readonly provider?: ServerProvider | undefined;
  readonly logger?: Partial<Pick<Logger, 'info'>> | undefined;
}

export async function createServer(values: unknown = {},
  { provider, logger }: ServerDependencies = {}): Promise<FramerizeInstance> {
  const config = loadConfig(values);
  const token = await readServerToken(config);
  await loadAccounts(config.accountsFile);
  const backend: InferenceBackend = {
    async ready(context) {
      if (!provider?.forward || !provider?.ready) return { ready: false, reason: 'configuration_unavailable' };
      return provider.ready(context);
    },
    async forward(request) {
      if (!provider?.forward) throw new ProviderError('Provider unavailable', { code: 'provider_unavailable', status: 503 });
      return provider.forward(request);
    },
    close() { return provider?.close?.(); },
  };
  const { host, port, maxBodyBytes, headersTimeoutMs, inferenceTimeoutMs, idleTimeoutMs,
    requestHeadersTimeoutMs, shutdownTimeoutMs } = config;
  const app = createInstance({ backend, token: token || null, host, port, maxBodyBytes,
    authentication: token ? 'required' : 'none',
    headersTimeoutMs, inferenceTimeoutMs, idleTimeoutMs, requestHeadersTimeoutMs, shutdownTimeoutMs,
    onRequest(event) { return logger?.info?.('request.finished', event); },
  });
  app.server.prependListener('request', (request, response) => {
    if (request.url === '/ready') response.setHeader('x-framerize-server-pid', String(process.pid));
  });
  if (config.privateNetworkOnly) {
    app.server.prependListener('connection', socket => {
      if (!acceptsPrivateNetworkConnection(socket)) socket.destroy();
    });
  }
  return app;
}
