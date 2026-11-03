import http from 'node:http';
import https from 'node:https';
import { classifyReadiness } from './contract.js';

const MESSAGES = Object.freeze({
  OPTIONS: 'invalid readiness options',
  ABORTED: 'readiness request aborted',
  HEADERS_TIMEOUT: 'readiness headers timeout',
  IDLE_TIMEOUT: 'readiness body idle timeout',
  TIMEOUT: 'readiness exchange timeout',
  SIZE: 'readiness response exceeds byte limit',
  RESPONSE: 'invalid readiness response',
  NETWORK: 'readiness request failed',
});
const ALLOWED_HEADERS = ['content-type', 'content-encoding', 'content-length',
  'cache-control', 'retry-after', 'x-request-id'];
const readAborted = Object.getOwnPropertyDescriptor(AbortSignal.prototype, 'aborted').get;
const failure = (kind) => Object.assign(new Error(MESSAGES[kind]), { code: `ERR_READINESS_${kind}` });

function settings(options) {
  try {
    if (!options || typeof options !== 'object' || Array.isArray(options)) throw failure('OPTIONS');
    const { upstream, remoteToken, signal, headersTimeoutMs = 15000,
      idleTimeoutMs = 5000, readinessTimeoutMs = 20000, maxReadinessBytes = 65536 } = options;
    if (typeof upstream !== 'string' || typeof remoteToken !== 'string') throw failure('OPTIONS');
    const url = new URL(upstream);
    if (!['http:', 'https:'].includes(url.protocol) || url.origin !== upstream) throw failure('OPTIONS');
    if (!/^[A-Za-z0-9._~+/-]+=*$/.test(remoteToken) || /\s/.test(remoteToken)) throw failure('OPTIONS');
    const aborted = signal === undefined ? false : readAborted.call(signal);
    for (const limit of [headersTimeoutMs, idleTimeoutMs, readinessTimeoutMs, maxReadinessBytes]) {
      if (!Number.isSafeInteger(limit) || limit <= 0) throw failure('OPTIONS');
    }
    if ([headersTimeoutMs, idleTimeoutMs, readinessTimeoutMs].some((limit) => limit > 2147483647)) throw failure('OPTIONS');
    return { url, remoteToken, signal, aborted, headersTimeoutMs, idleTimeoutMs, readinessTimeoutMs, maxReadinessBytes };
  } catch { throw failure('OPTIONS'); }
}

function responseHeaders(headers) {
  const nominated = new Set((headers.connection || '').split(',').map((name) => name.trim().toLowerCase()));
  return Object.fromEntries(ALLOWED_HEADERS.filter((name) => !nominated.has(name) && headers[name] !== undefined)
    .map((name) => [name, headers[name]]));
}

export async function probeReadiness(options) {
  const config = settings(options);
  if (config.aborted) throw failure('ABORTED');
  return new Promise((resolve, reject) => {
    let request, response, agent, headerTimer, idleTimer, wholeTimer;
    let settled = false, delivered = false, error, result, length = 0;
    const chunks = [];
    function deliver() {
      if (delivered) return;
      delivered = true;
      request?.off('error', onNetworkError);
      request?.off('close', onRequestClose);
      response?.off('error', onResponseError);
      response?.off('aborted', onResponseError);
      response?.off('close', onResponseClose);
      if (error) reject(error); else resolve(result);
    }
    function finish(problem, value) {
      if (settled) return;
      settled = true;
      error = problem;
      result = value;
      for (const timer of [headerTimer, idleTimer, wholeTimer]) clearTimeout(timer);
      if (config.signal) EventTarget.prototype.removeEventListener.call(config.signal, 'abort', onAbort);
      request?.off('response', onResponse);
      request?.off('upgrade', onUpgrade);
      response?.off('data', onData);
      response?.off('end', onEnd);
      chunks.length = 0;
      response?.destroy();
      request?.destroy();
      agent?.destroy();
      if (!request) deliver();
    }
    function onAbort() { finish(failure('ABORTED')); }
    function onNetworkError() { finish(failure('NETWORK')); }
    function onResponseError() { finish(failure('RESPONSE')); }
    function onRequestClose() {
      if (!settled) finish(failure('RESPONSE'));
      deliver();
    }
    function onResponseClose() { if (!response.complete) onResponseError(); }
    function armIdle() {
      clearTimeout(idleTimer);
      idleTimer = setTimeout(() => finish(failure('IDLE_TIMEOUT')), config.idleTimeoutMs);
    }
    function onData(chunk) {
      if (settled) return;
      length += chunk.length;
      if (length > config.maxReadinessBytes) return finish(failure('SIZE'));
      chunks.push(chunk);
      armIdle();
    }
    function onEnd() {
      if (!response.complete) return onResponseError();
      const body = Buffer.concat(chunks, length);
      const encoding = response.headers['content-encoding'];
      const readiness = !encoding || encoding.trim().toLowerCase() === 'identity'
        ? classifyReadiness(response.statusCode, body) : Object.freeze({ state: 'malformed' });
      finish(undefined, { status: response.statusCode, headers: responseHeaders(response.headers), body, readiness });
    }
    function onResponse(incoming) {
      response = incoming;
      clearTimeout(headerTimer);
      response.on('error', onResponseError);
      response.on('aborted', onResponseError);
      response.on('close', onResponseClose);
      const declared = response.headers['content-length'];
      if (declared !== undefined && Number(declared) > config.maxReadinessBytes) return finish(failure('SIZE'));
      armIdle();
      response.on('data', onData);
      response.once('end', onEnd);
    }
    function onUpgrade(incoming, socket) {
      socket.destroy();
      finish(failure('RESPONSE'));
    }
    wholeTimer = setTimeout(() => finish(failure('TIMEOUT')), config.readinessTimeoutMs);
    headerTimer = setTimeout(() => finish(failure('HEADERS_TIMEOUT')), config.headersTimeoutMs);
    if (config.signal) EventTarget.prototype.addEventListener.call(config.signal, 'abort', onAbort, { once: true });
    try {
      const transport = config.url.protocol === 'https:' ? https : http;
      agent = new transport.Agent({ keepAlive: false, proxyEnv: {} });
      request = transport.request(config.url, {
        agent, method: 'GET', path: '/ready', maxHeaderSize: 16384,
        headers: { authorization: `Bearer ${config.remoteToken}`,
          accept: 'application/json', 'accept-encoding': 'identity' },
      });
      request.maxHeadersCount = 0;
      request.on('error', onNetworkError);
      request.once('close', onRequestClose);
      request.once('response', onResponse);
      request.once('upgrade', onUpgrade);
      request.end();
    } catch { finish(failure('NETWORK')); }
  });
}
