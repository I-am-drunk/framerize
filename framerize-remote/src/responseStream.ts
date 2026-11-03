import type { IncomingMessage } from 'node:http';

export function responseStream(
  message: IncomingMessage, dispose: () => void, signal: AbortSignal,
): AsyncIterable<Buffer> {
  let disposed = false;
  const release = (): void => {
    if (disposed) return;
    disposed = true;
    signal.removeEventListener('abort', release);
    message.off('close', release);
    dispose();
  };
  signal.addEventListener('abort', release, { once: true });
  message.once('close', release);
  if (signal.aborted || message.destroyed) release();
  return {
    [Symbol.asyncIterator](): AsyncIterator<Buffer> {
      const inner = message[Symbol.asyncIterator]() as AsyncIterator<Buffer>;
      return {
        async next(): Promise<IteratorResult<Buffer>> {
          try {
            const result = await inner.next();
            if (result.done) release();
            return result;
          } catch { release(); throw new Error('remote response failed'); }
        },
        async return(): Promise<IteratorResult<Buffer>> {
          release();
          if (inner.return) await inner.return();
          return { done: true, value: undefined };
        },
      };
    },
  };
}
