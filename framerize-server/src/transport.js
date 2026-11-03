export function appError(status, code, message) {
  return Object.assign(new Error(message), { status, code });
}

export function bounded(work, controller, milliseconds, error) {
  const { signal } = controller;
  if (signal.aborted) return Promise.reject(signal.reason);
  return new Promise((resolve, reject) => {
    const aborted = () => { cleanup(); reject(signal.reason); };
    const timer = setTimeout(() => controller.abort(error), milliseconds);
    const cleanup = () => { clearTimeout(timer); signal.removeEventListener('abort', aborted); };
    signal.addEventListener('abort', aborted, { once: true });
    Promise.resolve().then(() => {
      signal.throwIfAborted();
      return work();
    }).then((value) => { cleanup(); resolve(value); }, (failure) => { cleanup(); reject(failure); });
  });
}

export function finishIterator(iterator) {
  try { Promise.resolve(iterator.return?.()).catch(() => {}); } catch {}
}

export async function readBody(request, controller, config) {
  const tooLarge = () => appError(413, 'body_too_large', 'Request body too large');
  if (Number(request.headers['content-length']) > config.maxBodyBytes) throw tooLarge();
  const iterator = request.iterator({ destroyOnReturn: false });
  const chunks = [];
  let size = 0;
  try {
    while (true) {
      const { value, done } = await bounded(() => iterator.next(), controller, config.idleTimeoutMs,
        appError(408, 'idle_timeout', 'Request body timed out'));
      if (done) break;
      size += value.length;
      if (size > config.maxBodyBytes) throw tooLarge();
      chunks.push(value);
    }
  } finally { finishIterator(iterator); }
  const body = Buffer.concat(chunks, size);
  try {
    const parsed = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(body));
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error();
  } catch { throw appError(400, 'invalid_json', 'Invalid JSON body'); }
  return body;
}

export function drain(response, controller, timeout) {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timer);
      response.off('drain', done);
      response.off('close', closed);
      controller.signal.removeEventListener('abort', aborted);
    };
    const done = () => { cleanup(); resolve(); };
    const aborted = () => { cleanup(); reject(controller.signal.reason); };
    const closed = () => { cleanup(); reject(appError(499, 'client_disconnected', 'Client disconnected')); };
    const timer = setTimeout(() => controller.abort(appError(504, 'idle_timeout', 'Client idle timeout')), timeout);
    response.once('drain', done);
    response.once('close', closed);
    controller.signal.addEventListener('abort', aborted, { once: true });
    if (controller.signal.aborted) aborted();
    else if (response.destroyed) closed();
  });
}
