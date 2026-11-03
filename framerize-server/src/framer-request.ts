import { randomUUID } from 'node:crypto';
import { ProviderError } from './errors.js';
import type { InferenceRequest } from '../dist/core.js';

export const FRAMER_ROUTES = Object.freeze({
  '/v1/responses': '/ai/v3/openai/responses/',
  '/v1/chat/completions': '/ai/v3/chat/',
});

export interface FramerRequest {
  readonly request: Readonly<Record<string, unknown>>;
}

function invalid(): never {
  throw new ProviderError('Invalid inference request', { code: 'invalid_request_body', status: 400 });
}

export function decodeFramerRequest(path: InferenceRequest['path'], bytes: Buffer): FramerRequest {
  let value: unknown;
  try { value = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)); }
  catch { invalid(); }
  if (!value || typeof value !== 'object' || Array.isArray(value)) invalid();
  const request = value as Record<string, unknown>;
  if (typeof request.model !== 'string' || !request.model.trim() || request.model.length > 256) invalid();
  if (request.stream !== undefined && typeof request.stream !== 'boolean') invalid();
  if (path === '/v1/responses') return { request };
  if (path !== '/v1/chat/completions' || !Array.isArray(request.messages)) invalid();
  return { request };
}

export function encodeFramerRequest(request: FramerRequest, projectId: string, castleToken: string): Buffer {
  if (typeof projectId !== 'string' || !/^[A-Za-z0-9_-]{1,128}$/.test(projectId) ||
      typeof castleToken !== 'string' || !castleToken || castleToken.length > 65_536) {
    throw new ProviderError('Provider credentials unavailable', { code: 'credentials_unavailable', status: 503 });
  }
  return Buffer.from(JSON.stringify({ projectId, featureCategory: 'agents', featureName: 'agents',
    model: request.request.model, step: 'message', sessionId: randomUUID(), stepIndex: 0,
    turnId: randomUUID(), userPrompt: userPrompt(request.request), castleToken, request: request.request,
  }));
}

function userPrompt(request: Readonly<Record<string, unknown>>): string {
  const input = request.messages ?? request.input;
  if (typeof input === 'string') return [...input].slice(0, 300).join('');
  const message = lastUserMessage(input);
  const content = message?.content;
  const text = typeof content === 'string' ? content : Array.isArray(content)
    ? content.map(item => typeof item?.text === 'string' ? item.text
      : typeof item?.input_text === 'string' ? item.input_text : '').join(' ') : '';
  return [...text].slice(0, 300).join('');
}

function lastUserMessage(input: unknown): Record<string, unknown> | undefined {
  if (!Array.isArray(input)) return undefined;
  for (let index = input.length - 1; index >= 0; index -= 1) {
    const item: unknown = input[index];
    if (item && typeof item === 'object' && 'role' in item && item.role === 'user') {
      return item as Record<string, unknown>;
    }
  }
  return undefined;
}
