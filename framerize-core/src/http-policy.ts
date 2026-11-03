import { createHash, timingSafeEqual } from 'node:crypto';
import type { IncomingMessage } from 'node:http';

export const routes = {
  '/health': 'GET', '/ready': 'GET',
  '/v1/responses': 'POST', '/v1/chat/completions': 'POST',
} as const;
export type Route = keyof typeof routes;
export const capabilities = { responses: true, chatCompletions: true, streaming: true };
export const requestHeaders = ['accept', 'content-type', 'openai-beta'];
export const responseHeaders = ['content-type', 'content-encoding', 'cache-control',
  'retry-after', 'x-request-id'];
export const hashToken = (token: string): Buffer => createHash('sha256').update(token).digest();

export function authenticated(request: IncomingMessage, tokenHash: Buffer | null): boolean {
  if (tokenHash === null) return false;
  const count = request.rawHeaders.filter((value, index) =>
    index % 2 === 0 && value.toLowerCase() === 'authorization').length;
  if (count !== 1 || typeof request.headers.authorization !== 'string') return false;
  const match = /^Bearer ([A-Za-z0-9._~+/-]+=*)$/i.exec(request.headers.authorization);
  return Boolean(match?.[1] && timingSafeEqual(tokenHash, hashToken(match[1])));
}

export function selectedHeaders(headers: Readonly<Record<string, unknown>>,
  names: readonly string[]): Record<string, string> {
  const normalized = Object.fromEntries(Object.entries(headers)
    .map(([name, value]) => [name.toLowerCase(), value]));
  const connection = String(normalized.connection ?? '').toLowerCase().split(',')
    .map((name) => name.trim());
  return Object.fromEntries(names.flatMap((name) => {
    const value = normalized[name];
    return typeof value === 'string' && !connection.includes(name) && !/[\r\n\0]/.test(value)
      ? [[name, value]] : [];
  }));
}
