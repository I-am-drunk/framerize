import assert from 'node:assert/strict';
import { request, type IncomingHttpHeaders, type RequestOptions } from 'node:http';
import { connect } from 'node:net';
import type { InferenceBackend } from '../src/backend.js';

export const token = 'synthetic-local-token';
export const authorization = { authorization: `Bearer ${token}` };
export function deferred<Value>() {
  let resolve!: (value: Value) => void;
  const promise = new Promise<Value>((fulfilled) => { resolve = fulfilled; });
  return { promise, resolve };
}

export const body = async function* (value = '{"output":"synthetic"}') {
  yield Buffer.from(value);
};
export const backend = (changes: Partial<InferenceBackend> = {}): InferenceBackend => ({
  async ready() { return true; },
  async forward() { return { status: 200, headers: { 'content-type': 'application/json' }, body: body() }; },
  close() {}, ...changes,
});

export interface Reply { status: number; headers: IncomingHttpHeaders; body: Buffer; }
export function comparable(reply: Reply) {
  const { date, connection, 'keep-alive': keepAlive, ...headers } = reply.headers;
  return { status: reply.status, headers, body: reply.body.toString('utf8') };
}

export async function rawExchange(port: number, payload: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const socket = connect({ host: '127.0.0.1', port });
    const chunks: Buffer[] = [];
    socket.setTimeout(3000, () => socket.destroy(new Error('Raw fixture timed out')));
    socket.on('connect', () => socket.write(payload));
    socket.on('data', (chunk: Buffer) => chunks.push(chunk));
    socket.on('error', reject);
    socket.on('close', () => resolve(Buffer.concat(chunks).toString('utf8')));
  });
}

export async function unavailable(port: number): Promise<void> {
  await assert.rejects(exchange(port, '/health'), { code: 'ECONNREFUSED' });
}

export async function exchange(port: number, path: string, options: RequestOptions = {},
  bytes = Buffer.from('{}')): Promise<Reply> {
  return new Promise((resolve, reject) => {
    const client = request({ ...options, host: '127.0.0.1', port, path, agent: false }, (response) => {
      const chunks: Buffer[] = [];
      response.on('data', (chunk: Buffer) => chunks.push(chunk));
      response.on('error', reject);
      response.on('end', () => resolve({ status: response.statusCode!,
        headers: response.headers, body: Buffer.concat(chunks) }));
    });
    client.on('error', reject);
    client.setTimeout(3000, () => client.destroy(new Error('Fixture exchange timed out')));
    client.end(options.method === 'POST' ? bytes : undefined);
  });
}
