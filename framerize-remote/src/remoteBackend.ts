import type {
  BackendReadiness, InferenceBackend, InferenceRequest, InferenceResponse, RequestContext,
} from '@framerize/core';
import { classifyReadiness } from '@framerize/core';
import { sendRequest, type RawResponse } from './remoteClient.js';
import { readBounded } from './readBounded.js';
import { responseStream } from './responseStream.js';
import { FORWARDED_REQUEST_HEADERS, type RemoteBackendConfig } from './backendConfig.js';
import { networkDiagnostic, type DiagnosticCode } from './diagnostics.js';

export interface ReadinessDiagnosis {
  readonly readiness: BackendReadiness;
  readonly code?: DiagnosticCode;
}

export interface RemoteBackend extends InferenceBackend {
  diagnose(context: RequestContext): Promise<ReadinessDiagnosis>;
}

function unavailable(code: DiagnosticCode): ReadinessDiagnosis {
  return { readiness: { ready: false, reason: 'upstream_unavailable' }, code };
}

function pick(headers: Readonly<Record<string, string>>): Record<string, string> {
  const result: Record<string, string> = {};
  for (const name of FORWARDED_REQUEST_HEADERS) {
    const value = headers[name];
    if (typeof value === 'string') result[name] = value;
  }
  return result;
}

export function createRemoteBackend(config: RemoteBackendConfig): RemoteBackend {
  for (const value of [config.headersTimeoutMs, config.idleTimeoutMs,
    config.readinessBodyMaxBytes, config.readinessWholeMs]) {
    if (!Number.isSafeInteger(value) || value <= 0 || value > 2_147_483_647) {
      throw new Error('invalid remote limit');
    }
  }
  if (config.readinessBodyMaxBytes > 65536) throw new Error('invalid readiness byte limit');
  const active = new Set<AbortController>();
  let closed = false;
  function track(parent: AbortSignal) {
    const controller = new AbortController();
    const relay = (): void => controller.abort();
    active.add(controller);
    if (parent.aborted) relay();
    else parent.addEventListener('abort', relay, { once: true });
    return { signal: controller.signal, abort: relay, release() {
      active.delete(controller);
      parent.removeEventListener('abort', relay);
    } };
  }
  async function ready(context: RequestContext): Promise<BackendReadiness> {
    return (await diagnose(context)).readiness;
  }
  async function diagnose(context: RequestContext): Promise<ReadinessDiagnosis> {
    if (closed) return { readiness: { ready: false, reason: 'configuration_unavailable' },
      code: 'configuration_unavailable' };
    const operation = track(context.signal);
    let expired = false;
    const deadline = setTimeout(() => { expired = true; operation.abort(); }, config.readinessWholeMs);
    let raw: RawResponse | undefined;
    try {
      raw = await sendRequest({
        origin: config.endpoint.origin, path: '/ready', method: 'GET',
        token: config.serverToken, headers: { accept: 'application/json' },
        signal: operation.signal, headersTimeoutMs: config.headersTimeoutMs,
        idleTimeoutMs: config.idleTimeoutMs,
      });
      const status = raw.message.statusCode ?? 0;
      const { bytes } = await readBounded(raw.message, config.readinessBodyMaxBytes,
        config.idleTimeoutMs, operation.signal);
      const result = classifyReadiness(raw.message.statusCode ?? 0, bytes);
      if (result.state === 'ready') return { readiness: true };
      const reason = result.state === 'unavailable' && result.reason ? result.reason : 'upstream_unavailable';
      const code = status === 401 || status === 403 ? 'remote_auth'
        : (status < 200 || status >= 300) && status !== 503 ? 'http_refused'
        : result.state === 'malformed' ? 'malformed_response'
        : result.state === 'incompatible' ? 'incompatible_response' : reason;
      return { readiness: { ready: false, reason }, code };
    } catch (error) {
      return unavailable(context.signal.aborted ? 'aborted' : expired ? 'readiness_timeout' : networkDiagnostic(error));
    } finally {
      clearTimeout(deadline);
      raw?.dispose();
      operation.release();
    }
  }
  return {
    ready, diagnose,
    async forward(request: InferenceRequest): Promise<InferenceResponse> {
      if (closed) throw new Error('backend closed');
      if (request.path !== '/v1/responses' && request.path !== '/v1/chat/completions') {
        throw new Error('invalid inference path');
      }
      const operation = track(request.signal);
      let raw: RawResponse | undefined;
      const release = (): void => { raw?.dispose(); operation.release(); };
      try {
        const readiness = await ready({ signal: operation.signal });
        if (operation.signal.aborted || closed) throw new Error('request aborted');
        if (readiness !== true) {
          release();
          return { status: 503, headers: { 'content-type': 'application/json' },
            body: [Buffer.from(JSON.stringify({ error: { code: readiness.reason,
              message: 'Remote server is not ready' } }))] };
        }
        raw = await sendRequest({
          origin: config.endpoint.origin, path: request.path, method: 'POST',
          token: config.serverToken, headers: pick(request.headers), body: request.body,
          signal: operation.signal, headersTimeoutMs: config.headersTimeoutMs,
          idleTimeoutMs: config.idleTimeoutMs,
        });
        const headers: Record<string, string> = {};
        for (const [name, value] of Object.entries(raw.message.headers)) {
          if (typeof value === 'string') headers[name] = value;
          else if (Array.isArray(value)) headers[name] = value.join(', ');
        }
        return { status: raw.message.statusCode ?? 502, headers,
          body: responseStream(raw.message, release, operation.signal) };
      } catch {
        release();
        throw new Error('remote request failed');
      }
    },
    close(): void {
      closed = true;
      for (const controller of active) controller.abort();
      active.clear();
    },
  };
}
