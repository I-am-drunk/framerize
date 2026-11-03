import { createDeadline } from './sse.js';
import { prepareFramer } from './framer.js';
import { readCredential } from './config.js';

export function upstreamHeaders(headers, authorization, isFramer = false) {
  const result = {};
  for (const name of ['accept', 'content-type', 'openai-beta', 'anthropic-version', 'anthropic-beta']) {
    if (headers[name]) result[name] = headers[name];
  }
  if (authorization) result.authorization = authorization;
  if (isFramer) {
    result['content-type'] = 'application/json; charset=utf-8';
    result['x-requested-by'] = 'Framer';
  }
  result.accept = headers.accept || 'text/event-stream, application/json';
  return result;
}

export async function openUpstream({ config, request, body, account, signal, fetchImpl = fetch }) {
  if (!config.upstream && !account?.upstream) throw new Error('no upstream endpoint configured');
  const base = account?.upstream || config.upstream;
  const url = new URL(request.url, 'http://local');
  const deadline = createDeadline(signal, config.requestTimeoutMs);
  try {
    const framer = await prepareFramer({ account, path: url.pathname, body,
      signal: deadline.signal, fetchImpl });
    const authorization = framer?.authorization || (account?.authorizationEnv
      ? process.env[account.authorizationEnv] : await readCredential(account?.authorizationFile));
    const destination = new URL(framer?.url || `${base}${url.pathname}${url.search}`);
    const response = await fetchImpl(destination, {
      method: request.method,
      headers: upstreamHeaders(request.headers, authorization, Boolean(framer)),
      body: ['GET', 'HEAD'].includes(request.method) ? undefined : framer?.body || body,
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
