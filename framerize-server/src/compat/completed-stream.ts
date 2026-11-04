import type { InferenceResponse } from '../../dist/core.js';
import { ProviderError } from '../errors.js';
import { event } from './events.js';
import { messagesStream } from './messages-stream.js';
import { responsesStream } from './responses-stream.js';
import type { CompletedReplyProtocol } from './completed-reply.js';

export interface CompletedResponse extends InferenceResponse {
  readonly reply?: Record<string, unknown>;
}

type Outcome = { response: CompletedResponse } | { error: unknown };

function streamError(protocol: CompletedReplyProtocol, error: unknown): Buffer {
  const known = error instanceof ProviderError ? error : undefined;
  const code = known && ['upstream_http_error', 'upstream_response_invalid', 'headers_timeout',
    'idle_timeout'].includes(known.code) ? known.code : 'upstream_error';
  const message = 'Upstream request failed';
  return protocol === 'responses'
    ? event('error', { code, message, param: null, sequence_number: 0 })
    : event('error', { error: { type: errorTypes[known?.status ?? 502] ?? 'api_error', code, message } });
}

const errorTypes: Readonly<Record<number, string>> = {
  400: 'invalid_request_error', 401: 'authentication_error', 403: 'permission_error',
  404: 'not_found_error', 413: 'request_too_large', 429: 'rate_limit_error', 529: 'overloaded_error',
};

export function streamCompletedReply(pending: Promise<CompletedResponse>, protocol: CompletedReplyProtocol,
  signal: AbortSignal, heartbeatMs: number): AsyncGenerator<Buffer> {
  let outcome: Outcome | undefined;
  let wake = () => {};
  void pending.then(response => { outcome = { response }; wake(); }, error => { outcome = { error }; wake(); });
  return (async function* () {
    try {
      while (!outcome) {
        signal.throwIfAborted();
        yield Buffer.from(': keepalive\n\n');
        await new Promise<void>(resolve => {
          const finish = () => {
            clearTimeout(timer); signal.removeEventListener('abort', finish);
            wake = () => {}; resolve();
          };
          const timer = setTimeout(finish, heartbeatMs);
          wake = finish;
          signal.addEventListener('abort', finish, { once: true });
          if (outcome || signal.aborted) finish();
        });
      }
      signal.throwIfAborted();
      if ('error' in outcome) throw outcome.error;
      const { response } = outcome;
      if (!response.reply) throw new ProviderError('Upstream request failed', {
        status: response.status, code: 'upstream_http_error', retryable: false,
      });
      yield* protocol === 'responses' ? responsesStream(response.reply) : messagesStream(response.reply);
    } catch (error) {
      signal.throwIfAborted();
      yield streamError(protocol, error);
    }
  })();
}
