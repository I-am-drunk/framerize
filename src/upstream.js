import { createDeadline } from './sse.js';

export function upstreamHeaders(headers, authorization) {
  const result = {};
  for (const name of ['accept', 'content-type', 'openai-beta', 'anthropic-version', 'anthropic-beta']) {
    if (headers[name]) result[name] = headers[name];
  }
  if (authorization) result.authorization = authorization;
  result.accept = headers.accept || 'text/event-stream, application/json';
  return result;
}

export async function openUpstream({ config, request, body, account, signal, fetchImpl = fetch }) {
  if (!config.upstream && !account?.upstream) throw new Error('no upstream endpoint configured');
  const base = account?.upstream || config.upstream;
  const url = new URL(request.url, 'http://local');
  const destination = new URL(`${base}${url.pathname}${url.search}`);
  const deadline = createDeadline(signal, config.requestTimeoutMs);
  try {
    const response = await fetchImpl(destination, {
      method: request.method,
      headers: upstreamHeaders(request.headers, account?.authorizationEnv
        ? process.env[account.authorizationEnv] : undefined),
      body: ['GET', 'HEAD'].includes(request.method) ? undefined : body,
      signal: deadline.signal,
      redirect: 'error',
    });
    return { response, deadline };
  } catch (error) {
    deadline.clear();
    throw error;
  }
}

export { createDeadline };
