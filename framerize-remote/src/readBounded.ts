import type { IncomingMessage } from 'node:http';
import { RemoteDiagnosticError, networkDiagnostic } from './diagnostics.js';

export interface BoundedResult { readonly bytes: Buffer; }

export function readBounded(
  message: IncomingMessage, maxBytes: number, idleMs: number, signal: AbortSignal,
): Promise<BoundedResult> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let total = 0;
    let settled = false;
    let idle: NodeJS.Timeout;
    function done(error?: Error): void {
      if (settled) return;
      settled = true;
      clearTimeout(idle);
      signal.removeEventListener('abort', onAbort);
      message.off('data', onData);
      message.off('end', onEnd);
      message.off('error', onError);
      message.off('close', onClose);
      message.destroy();
      if (error) reject(error);
      else resolve({ bytes: Buffer.concat(chunks, total) });
    }
    const armIdle = (): void => {
      clearTimeout(idle);
      idle = setTimeout(() => done(new RemoteDiagnosticError('idle_timeout')), idleMs);
    };
    const onAbort = (): void => done(new RemoteDiagnosticError('aborted'));
    const onEnd = (): void => done();
    const onError = (error: unknown): void => done(new RemoteDiagnosticError(networkDiagnostic(error)));
    const onClose = (): void => done(new RemoteDiagnosticError('connection_failed'));
    const onData = (chunk: Buffer): void => {
      if (total + chunk.length > maxBytes) {
        done(new RemoteDiagnosticError('response_too_large'));
        return;
      }
      total += chunk.length;
      chunks.push(chunk);
      armIdle();
    };
    if (signal.aborted) return onAbort();
    if (message.destroyed) return onClose();
    signal.addEventListener('abort', onAbort, { once: true });
    message.on('data', onData);
    message.once('end', onEnd);
    message.once('error', onError);
    message.once('close', onClose);
    armIdle();
  });
}
