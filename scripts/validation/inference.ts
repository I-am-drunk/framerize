import { addAbortListener } from 'node:events';
import { sendRequest, type RawResponse } from '../../framerize-remote/src/remoteClient.js';
import { readBounded } from '../../framerize-remote/src/readBounded.js';
import { parseRemoteEndpoint } from '../../framerize-remote/src/remoteUrl.js';
import { RemoteDiagnosticError } from '../../framerize-remote/src/diagnostics.js';
import { inspectCompletion } from './completion.js';

export interface InferenceCheckOptions {
  readonly endpoint: string;
  readonly token: string;
  readonly model: string;
  readonly timeoutMs?: number;
  readonly signal?: AbortSignal;
}

export function validateInferenceOptions(input: InferenceCheckOptions) {
  const { endpoint, token, model, signal, timeoutMs = 30000 } = input;
  const origin = parseRemoteEndpoint(endpoint, false).origin;
  if (typeof token !== 'string' || token.length > 8192 || !/^[A-Za-z0-9._~+/-]+=*$/.test(token)
    || typeof model !== 'string' || !model.trim() || model.length > 128 || /[\x00-\x1f\x7f]/.test(model)
    || !Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 60000) throw new Error();
  if (signal !== undefined) Reflect.get(AbortSignal.prototype, 'aborted', signal);
  return { origin, token, model, signal, timeoutMs };
}

type Failure = 'configuration' | 'aborted' | 'timeout' | 'transport' | 'http_refused'
  | 'invalid_stream' | 'response_too_large' | 'unsupported_encoding';

export async function checkInference(input: InferenceCheckOptions) {
  const started = performance.now();
  let requestAttempts = 0;
  let status: number | null = null;
  const summary = () => ({ scope: 'single-endpoint-text-inference', requestAttempts, status,
    durationMs: Math.round(performance.now() - started) });
  const fail = (code: Failure) => ({ ...summary(), ok: false as const, code });
  let config: ReturnType<typeof validateInferenceOptions>;
  try { config = validateInferenceOptions(input); } catch { return fail('configuration'); }
  if (config.signal?.aborted) return fail('aborted');
  const controller = new AbortController();
  const cancellation = config.signal ? addAbortListener(config.signal, () => controller.abort()) : undefined;
  let expired = false;
  const deadline = setTimeout(() => { expired = true; controller.abort(); }, config.timeoutMs);
  let raw: RawResponse | undefined;
  try {
    if (controller.signal.aborted) return fail('aborted');
    const body = Buffer.from(JSON.stringify({ model: config.model, stream: true,
      messages: [{ role: 'user', content: 'Reply with exactly OK.' }] }));
    requestAttempts = 1;
    raw = await sendRequest({ origin: config.origin, path: '/v1/chat/completions', method: 'POST',
      token: config.token, headers: { accept: 'text/event-stream', 'content-type': 'application/json' },
      body: (async function* () { yield body; })(), signal: controller.signal,
      headersTimeoutMs: config.timeoutMs, idleTimeoutMs: 5000 });
    status = raw.message.statusCode ?? 0;
    if (status !== 200) return fail('http_refused');
    const headers = raw.message.headers;
    if (!/^text\/event-stream(?:\s*;|$)/i.test(headers['content-type'] ?? '')) return fail('invalid_stream');
    if (headers['content-encoding'] && headers['content-encoding'] !== 'identity') return fail('unsupported_encoding');
    const { bytes } = await readBounded(raw.message, 65536, 5000, controller.signal);
    let completion: ReturnType<typeof inspectCompletion>;
    try { completion = inspectCompletion(bytes); } catch { return fail('invalid_stream'); }
    return { ...summary(), ok: true as const, ...completion, responseBytes: bytes.length };
  } catch (error) {
    return fail(config.signal?.aborted ? 'aborted' : expired ? 'timeout'
      : error instanceof RemoteDiagnosticError && error.code === 'response_too_large'
        ? 'response_too_large' : 'transport');
  } finally {
    clearTimeout(deadline);
    cancellation?.[Symbol.dispose]();
    raw?.dispose();
  }
}
