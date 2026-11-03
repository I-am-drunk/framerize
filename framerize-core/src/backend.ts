import type { Buffer } from 'node:buffer';
import type { ReadinessReason } from './readiness.js';

export type InferencePath = '/v1/responses' | '/v1/chat/completions';

export interface RequestContext {
  readonly signal: AbortSignal;
}

export interface InferenceRequest extends RequestContext {
  readonly path: InferencePath;
  readonly headers: Readonly<Record<string, string>>;
  readonly body: AsyncIterable<Buffer>;
}

export interface InferenceResponse {
  readonly status: number;
  readonly headers: Readonly<Record<string, string>>;
  readonly body: AsyncIterable<Buffer> | Iterable<Buffer>;
}

export type BackendReadiness = true | Readonly<{
  ready: false;
  reason: ReadinessReason;
}>;

export interface InferenceBackend {
  forward(request: InferenceRequest): Promise<InferenceResponse>;
  ready(context: RequestContext): Promise<BackendReadiness>;
  close(): void | Promise<void>;
}
