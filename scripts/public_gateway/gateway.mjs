import http from 'node:http';
import { pathToFileURL } from 'node:url';

const routes = new Map();
for (const name of ['responses', 'chat/completions', 'models']) {
  for (const prefix of ['', '/v1', '/openai/v1'])
    routes.set(`${prefix}/${name}`, `/openai/v1/${name}`);
}
for (const prefix of ['', '/v1', '/anthropic/v1'])
  routes.set(`${prefix}/messages`, '/anthropic/v1/messages');
routes.set('/anthropic/v1/models', '/anthropic/v1/models');
routes.set('/health', '/api/health');

export function createGateway({ target = 'http://127.0.0.1:8787', limit = 2,
  maxBody = 16 * 1024 * 1024 } = {}) {
  let active = 0;
  return http.createServer((req, res) => {
    res.setHeader('access-control-allow-origin', '*');
    res.setHeader('access-control-expose-headers', 'retry-after,x-should-retry');
    res.setHeader('cache-control', 'no-store');
    const reply = (status, message) => {
      if (res.destroyed || res.writableEnded) return;
      res.writeHead(status, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ error: { message } }));
    };
    let path;
    try { path = new URL(req.url, 'http://localhost').pathname; }
    catch { reply(400, 'The request URL is invalid.'); return; }
    if (path === '/' && req.method === 'GET') {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ service: 'Framerize', openai: '/openai/v1',
        anthropic: '/anthropic', shared: '/' })); return;
    }
    let route = routes.get(path);
    if (!route) { reply(404, 'This public endpoint exposes inference routes only.'); return; }
    if (['/models', '/v1/models'].includes(path) && req.headers['anthropic-version'])
      route = '/anthropic/v1/models';
    const method = route.endsWith('/models') || path === '/health' ? 'GET' : 'POST';
    if (req.method === 'OPTIONS') {
      res.writeHead(204, { 'access-control-allow-methods': method,
        'access-control-allow-headers': req.headers['access-control-request-headers'] || 'content-type,authorization,x-api-key,anthropic-version,anthropic-beta',
        'access-control-max-age': '600' }); res.end(); return;
    }
    if (req.method !== method) { reply(405, `This route requires ${method}.`); return; }
    if (active >= limit) {
      res.setHeader('retry-after', '2');
      reply(503, `The public gateway already has ${limit} active requests. Retry when one finishes.`); return;
    }
    if (Number(req.headers['content-length'] || 0) > maxBody) {
      reply(413, `The request body exceeds the public gateway limit of ${maxBody} bytes.`); return;
    }
    active++;
    let done = false, bytes = 0, remote;
    const finish = () => { if (!done) { done = true; active--; } };
    res.once('finish', finish);
    res.once('close', () => { if (!res.writableFinished) remote?.destroy(); finish(); });
    const headers = {};
    for (const key of ['content-type', 'content-length', 'content-encoding',
      'accept', 'accept-encoding', 'anthropic-version', 'anthropic-beta'])
      if (req.headers[key]) headers[key] = req.headers[key];
    remote = http.request(new URL(route, target), { method, headers }, upstream => {
      const outgoing = {};
      for (const key of ['content-type', 'content-length', 'content-encoding', 'retry-after', 'x-should-retry'])
        if (upstream.headers[key]) outgoing[key] = upstream.headers[key];
      res.writeHead(upstream.statusCode, outgoing); res.flushHeaders();
      upstream.on('error', error => {
        if (!res.destroyed)
          console.error(`Upstream response ended before completion: ${error.code || error.name}`);
        res.destroy();
      });
      upstream.pipe(res);
    });
    remote.setTimeout(1200000, () => remote.destroy(Object.assign(
      new Error('upstream idle timeout'), { code: 'ETIMEDOUT' })));
    remote.on('error', error => {
      if (res.headersSent) { res.destroy(); return; }
      const cause = /^[A-Z_0-9]+$/.test(error.code || '') ? error.code : 'transport error';
      reply(502, `The connection to the inference service failed: ${cause}.`);
    });
    req.on('aborted', () => remote.destroy());
    req.on('data', chunk => {
      bytes += chunk.length;
      if (bytes > maxBody) {
        req.unpipe(remote); remote.destroy();
        reply(413, `The request body exceeds the public gateway limit of ${maxBody} bytes.`);
      }
    });
    req.pipe(remote);
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const server = createGateway();
  server.listen(8790, '127.0.0.1', () => console.log('Public inference gateway listening on 127.0.0.1:8790'));
  process.once('SIGTERM', () => server.close());
}
