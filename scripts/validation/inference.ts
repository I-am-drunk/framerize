import { addAbortListener } from 'node:events';
import { sendRequest, type RawResponse } from '../../framerize-remote/src/remoteClient.js';
import { readBounded } from '../../framerize-remote/src/readBounded.js';
import { parseRemoteEndpoint } from '../../framerize-remote/src/remoteUrl.js';
import { RemoteDiagnosticError } from '../../framerize-remote/src/diagnostics.js';
import { inspectCompletion, inspectJsonCompletion } from './completion.js';

export interface InferenceCheckOptions {
  readonly endpoint: string;
  readonly token?: string | null;
  readonly model: string;
  readonly timeoutMs?: number;
  readonly signal?: AbortSignal;
  readonly protocol?: 'chat' | 'responses' | 'messages';
}

export function validateInferenceOptions(input: InferenceCheckOptions) {
  const { endpoint, token = null, model, signal, timeoutMs = 30000, protocol = 'chat' } = input;
  const origin = parseRemoteEndpoint(endpoint, false).origin;
  if ((token !== null && (typeof token !== 'string' || token.length > 8192 || !/^[A-Za-z0-9._~+/-]+=*$/.test(token)))
    || typeof model !== 'string' || !model.trim() || model.length > 128 || /[\x00-\x1f\x7f]/.test(model)
    || !Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 60000
    || !['chat', 'responses', 'messages'].includes(protocol)) throw new Error();
  if (signal !== undefined) Reflect.get(AbortSignal.prototype, 'aborted', signal);
  return { origin, token, model, signal, timeoutMs, protocol };
}

type Failure = 'configuration' | 'aborted' | 'timeout' | 'transport' | 'http_refused'
  | 'invalid_stream' | 'invalid_response' | 'response_too_large' | 'unsupported_encoding';

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
    const streaming = config.protocol === 'chat';
    const path = config.protocol === 'responses' ? '/openai/v1/responses'
      : config.protocol === 'messages' ? '/anthropic/v1/messages' : '/v1/chat/completions';
    const payload = { model: config.model, stream: streaming,
      ...(config.protocol === 'responses' ? { input: 'Reply with exactly OK.', max_output_tokens: 64 }
        : { messages: [{ role: 'user', content: 'Reply with exactly OK.' }],
          ...(streaming ? {} : { max_tokens: 64 }) }) };
    const body = Buffer.from(JSON.stringify(payload));
    const contentType = streaming ? 'text/event-stream' : 'application/json';
    requestAttempts = 1;
    raw = await sendRequest({ origin: config.origin, path, method: 'POST',
      token: config.token, headers: { accept: contentType, 'content-type': 'application/json' },
      body: (async function* () { yield body; })(), signal: controller.signal,
      headersTimeoutMs: config.timeoutMs, idleTimeoutMs: 5000 });
    status = raw.message.statusCode ?? 0;
    if (status !== 200) return fail('http_refused');
    const headers = raw.message.headers;
    const invalidReply = streaming ? 'invalid_stream' : 'invalid_response';
    if ((headers['content-type'] ?? '').split(';')[0]?.trim().toLowerCase() !== contentType) {
      return fail(invalidReply);
    }
    if (headers['content-encoding'] && headers['content-encoding'].toLowerCase() !== 'identity') return fail('unsupported_encoding');
    const { bytes } = await readBounded(raw.message, 65536, 5000, controller.signal);
    let completion: ReturnType<typeof inspectCompletion> | ReturnType<typeof inspectJsonCompletion>;
    try { completion = config.protocol === 'chat' ? inspectCompletion(bytes)
      : inspectJsonCompletion(config.protocol, bytes); } catch { return fail(invalidReply); }
    return { ...summary(), ok: true as const, protocol: config.protocol,
      ...completion, responseBytes: bytes.length };
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
