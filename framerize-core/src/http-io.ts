import { Buffer } from 'node:buffer';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { HttpError } from './http-errors.js';
import type { InferenceResponse } from './backend.js';

export function bounded<Value>(work: () => PromiseLike<Value>,
  controller: AbortController, milliseconds: number, failure: HttpError,
  discard?: (value: Value) => void): Promise<Value> {
  const { signal } = controller;
  if (signal.aborted) return Promise.reject(signal.reason);
  return new Promise((resolve, reject) => {
    let expiry: NodeJS.Immediate | undefined;
    const cleanup = () => {
      clearTimeout(timer);
      clearImmediate(expiry);
      signal.removeEventListener('abort', abort);
    };
    const abort = () => { cleanup(); reject(signal.reason); };
    const timer = setTimeout(() => { expiry = setImmediate(() => controller.abort(failure)); }, milliseconds);
    signal.addEventListener('abort', abort, { once: true });
    Promise.resolve().then(() => {
      signal.throwIfAborted();
      return work();
    }).then((value) => {
      cleanup();
      if (signal.aborted) {
        try { discard?.(value); } catch {}
        reject(signal.reason);
      } else resolve(value);
    },
      (error: unknown) => { cleanup(); reject(error); });
  });
}

export function bodyIterator(body: InferenceResponse['body']) {
  const asynchronous = body as AsyncIterable<Buffer>;
  return typeof asynchronous[Symbol.asyncIterator] === 'function'
    ? asynchronous[Symbol.asyncIterator]() : (body as Iterable<Buffer>)[Symbol.iterator]();
}

export function finish(iterator: AsyncIterator<Buffer> | Iterator<Buffer>): void {
  try { void Promise.resolve(iterator.return?.()).catch(() => {}); } catch {}
}

export async function readBody(request: IncomingMessage, controller: AbortController,
  maxBytes: number, idleMs: number): Promise<Buffer> {
  if (Number(request.headers['content-length']) > maxBytes) throw new HttpError('body_too_large');
  const iterator = request.iterator({ destroyOnReturn: false }) as AsyncIterator<Buffer>;
  const chunks: Buffer[] = [];
  let size = 0;
  try {
    while (true) {
      const next = await bounded(() => iterator.next(), controller, idleMs,
        new HttpError('request_timeout'));
      if (next.done) break;
      size += next.value.length;
      if (size > maxBytes) throw new HttpError('body_too_large');
      chunks.push(next.value);
    }
  } finally { finish(iterator); }
  const bytes = Buffer.concat(chunks, size);
  try {
    const parsed: unknown = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error();
  } catch { throw new HttpError('invalid_json'); }
  return bytes;
}

function drain(response: ServerResponse, controller: AbortController,
  idleMs: number): Promise<void> {
  return new Promise((resolve, reject) => {
    let expiry: NodeJS.Immediate | undefined;
    const cleanup = () => {
      clearTimeout(timer);
      clearImmediate(expiry);
      response.off('drain', done);
      response.off('close', closed);
      controller.signal.removeEventListener('abort', aborted);
    };
    const done = () => { cleanup(); resolve(); };
    const aborted = () => { cleanup(); reject(controller.signal.reason); };
    const closed = () => { cleanup(); reject(new HttpError('client_disconnected')); };
    const timer = setTimeout(() => {
      expiry = setImmediate(() => controller.abort(new HttpError('idle_timeout')));
    }, idleMs);
    response.once('drain', done);
    response.once('close', closed);
    controller.signal.addEventListener('abort', aborted, { once: true });
    if (controller.signal.aborted) aborted();
    else if (response.destroyed) closed();
  });
}

export async function streamBody(response: ServerResponse, body: InferenceResponse['body'],
  controller: AbortController, idleMs: number): Promise<void> {
  const iterator = bodyIterator(body);
  let complete = false;
  try {
    while (true) {
      const next = await bounded(() => Promise.resolve(iterator.next()), controller, idleMs,
        new HttpError('idle_timeout'));
      if (next.done) { complete = true; break; }
      if (!Buffer.isBuffer(next.value)) throw new HttpError('upstream_error');
      if (!response.write(next.value)) await drain(response, controller, idleMs);
    }
  } finally { if (!complete) finish(iterator); }
}
