import http from 'node:http';
import https from 'node:https';
import { ConfigurationError } from './errors.js';

export type FramerAuthCode = 'auth_aborted' | 'auth_closed' | 'auth_timeout' |
  'session_unavailable' | 'auth_refused' | 'auth_unavailable';
export class FramerAuthError extends Error {
  constructor(readonly code: FramerAuthCode) {
    super('Framer authentication unavailable');
    this.name = 'FramerAuthError';
  }
}
export interface FramerAuthOptions {
  readonly sessionCookie: string;
  readonly origin?: string;
  readonly timeoutMs?: number;
}
export interface FramerAuth {
  accessToken(context?: { readonly signal?: AbortSignal }): Promise<string>;
  invalidate(rejectedToken: string): void;
  close(): Promise<void>;
}

interface CachedToken { readonly bearer: string; readonly expiresAt: number }
interface Flight {
  readonly controller: AbortController;
  readonly promise: Promise<CachedToken>;
  waiters: number;
}

function joinFlight(flight: Flight, signal: AbortSignal | undefined, closed: () => boolean): Promise<string> {
  flight.waiters += 1;
  return new Promise((resolve, reject) => {
    let settled = false;
    const release = () => {
      if (settled) return false;
      settled = true;
      if (signal) EventTarget.prototype.removeEventListener.call(signal, 'abort', abort);
      flight.waiters -= 1;
      return true;
    };
    const abort = () => {
      if (!release()) return;
      const error = new FramerAuthError('auth_aborted');
      if (flight.waiters) reject(error);
      else {
        flight.controller.abort(error);
        void flight.promise.then(() => reject(error), () => reject(error));
      }
    };
    if (signal) EventTarget.prototype.addEventListener.call(signal, 'abort', abort, { once: true });
    flight.promise.then(value => {
      if (release()) closed() ? reject(new FramerAuthError('auth_closed')) : resolve(value.bearer);
    }, error => { if (release()) reject(error); });
  });
}
export function createFramerAuth(input: FramerAuthOptions): FramerAuth {
  const config = options(input);
  let cached: CachedToken | undefined;
  let active: Flight | undefined;
  let closed = false;
  let closing: Promise<void> | undefined;

  async function accessToken(context: { readonly signal?: AbortSignal } = {}): Promise<string> {
    let signal: AbortSignal | undefined;
    let aborted = false;
    try {
      signal = context.signal;
      if (signal !== undefined && !(signal instanceof AbortSignal)) throw new Error();
      aborted = signal?.aborted ?? false;
    } catch { throw new FramerAuthError('auth_unavailable'); }
    if (closed) throw new FramerAuthError('auth_closed');
    if (aborted) throw new FramerAuthError('auth_aborted');
    if (active?.controller.signal.aborted) {
      await active.promise.catch(() => {});
      return accessToken(signal ? { signal } : {});
    }
    if (cached && cached.expiresAt > Date.now() + 30_000) return cached.bearer;
    if (!active) {
      const controller = new AbortController();
      const flight: Flight = { controller, promise: acquire(config, controller.signal), waiters: 0 };
      active = flight;
      void flight.promise.then(value => {
        if (!closed && !controller.signal.aborted) cached = value;
        if (active === flight) active = undefined;
      }, () => { if (active === flight) active = undefined; });
    }
    return joinFlight(active, signal, () => closed);
  }

  return Object.freeze({
    accessToken,
    invalidate(rejectedToken: string) {
      if (cached?.bearer === rejectedToken) cached = undefined;
    },
    close() {
      if (closing) return closing;
      closed = true;
      cached = undefined;
      active?.controller.abort(new FramerAuthError('auth_closed'));
      closing = active ? active.promise.then(() => {}, () => {}) : Promise.resolve();
      return closing;
    },
  });
}

function options(input: FramerAuthOptions) {
  try {
    const origin = new URL(input.origin ?? 'https://api.framer.com');
    const cookie = input.sessionCookie;
    const timeout = input.timeoutMs ?? 15_000;
    if (origin.username || origin.password || origin.search || origin.hash || origin.pathname !== '/' ||
        origin.protocol !== 'https:' && !(origin.protocol === 'http:' && ['127.0.0.1', '[::1]'].includes(origin.hostname)) ||
        typeof cookie !== 'string' || !cookie.trim() || Buffer.byteLength(cookie) > 16_384 ||
        !Number.isSafeInteger(timeout) || timeout < 1 || timeout > 2_147_483_647) throw new Error();
    http.validateHeaderValue('cookie', cookie);
    return { url: new URL('/auth/ai/access-token', origin), cookie, timeout };
  } catch { throw new ConfigurationError('Invalid Framer authentication configuration'); }
}

function expiryTime(value: unknown): number {
  if (typeof value !== 'string') throw new Error();
  const match = /^(\d{4})-(0[1-9]|1[0-2])-(0[1-9]|[12]\d|3[01])T(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d+)?(?:Z|[+-](?:[01]\d|2[0-3]):[0-5]\d)$/.exec(value);
  if (!match) throw new Error();
  const year = Number(match[1]);
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  if (Number(match[3]) > days[Number(match[2]) - 1]!) throw new Error();
  return Date.parse(value);
}

function decode(bytes: Buffer): CachedToken {
  try {
    const value: unknown = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error();
    const { accessToken, expiresAt } = value as Record<string, unknown>;
    if (typeof accessToken !== 'string' || accessToken.length > 16_384 ||
        !/^[A-Za-z0-9._~+/-]+=*$/.test(accessToken)) throw new Error();
    const expiry = expiryTime(expiresAt);
    const now = Date.now();
    if (!Number.isFinite(expiry) || expiry <= now + 30_000) throw new Error();
    return { bearer: accessToken, expiresAt: Math.min(expiry, now + 600_000) };
  } catch { throw new FramerAuthError('auth_unavailable'); }
}

function acquire(config: ReturnType<typeof options>, signal: AbortSignal): Promise<CachedToken> {
  return new Promise((resolve, reject) => {
    const transport = config.url.protocol === 'https:' ? https : http;
    const request = transport.request(config.url, { method: 'GET', agent: false, rejectUnauthorized: true,
      maxHeaderSize: 16_384, headers: { accept: 'application/json', cookie: config.cookie } });
    request.maxHeadersCount = 0;
    let outcome: CachedToken | FramerAuthError | undefined;
    const fail = (code: FramerAuthCode) => {
      outcome ??= new FramerAuthError(code);
      request.destroy();
    };
    const abort = () => fail(signal.reason instanceof FramerAuthError
      && signal.reason.code === 'auth_closed' ? 'auth_closed' : 'auth_aborted');
    const timer = setTimeout(() => fail('auth_timeout'), config.timeout);
    request.on('error', () => fail('auth_unavailable'));
    request.once('close', () => {
      clearTimeout(timer);
      signal.removeEventListener('abort', abort);
      if (outcome && !(outcome instanceof FramerAuthError)) resolve(outcome);
      else reject(outcome ?? new FramerAuthError('auth_unavailable'));
    });
    request.once('upgrade', (response, socket) => { socket.destroy(); fail('auth_unavailable'); });
    request.once('response', response => {
      response.on('error', () => fail('auth_unavailable'));
      response.once('aborted', () => fail('auth_unavailable'));
      if (response.statusCode !== 200) {
        fail(response.statusCode === 401 ? 'session_unavailable'
          : response.statusCode === 403 ? 'auth_refused' : 'auth_unavailable');
        response.destroy();
        return;
      }
      const encoding = response.headers['content-encoding']?.toLowerCase();
      if (!/^application\/json(?:\s*;|$)/i.test(response.headers['content-type'] ?? '') ||
          encoding && encoding !== 'identity') {
        fail('auth_unavailable'); response.destroy(); return;
      }
      const chunks: Buffer[] = [];
      let size = 0;
      response.on('data', (chunk: Buffer) => {
        size += chunk.length;
        if (size > 65_536) { fail('auth_unavailable'); response.destroy(); }
        else chunks.push(chunk);
      });
      response.once('end', () => {
        if (!outcome) {
          try { outcome = decode(Buffer.concat(chunks)); }
          catch { outcome = new FramerAuthError('auth_unavailable'); }
        }
        request.destroy();
      });
    });
    if (signal.aborted) abort();
    else { signal.addEventListener('abort', abort, { once: true }); request.end(); }
  });
}
