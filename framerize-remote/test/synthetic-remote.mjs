import http from 'node:http';

export function startSyntheticRemote(options = {}) {
  const token = options.token ?? 'remote-server-token';
  const ready = options.ready ?? { apiVersion: 1, status: 'ready',
    capabilities: { responses: true, chatCompletions: true, streaming: true } };
  const received = { authorizations: [], paths: [], cancelled: 0, bodies: [] };

  const server = http.createServer((request, response) => {
    received.authorizations.push(request.headers.authorization ?? null);
    received.paths.push(request.url);
    const authorized = request.headers.authorization === `Bearer ${token}`;
    if (request.url === '/ready') {
      if (!authorized) return json(response, 401, { error: { code: 'unauthorized' } });
      return json(response, 200, ready);
    }
    if (!authorized) return json(response, 401, { error: { code: 'unauthorized' } });
    response.on('close', () => { if (!response.writableFinished) received.cancelled += 1; });
    const chunks = [];
    request.on('data', (chunk) => chunks.push(chunk));
    request.on('end', () => { received.bodies.push(Buffer.concat(chunks).toString('utf8')); });
    response.writeHead(200, { 'content-type': 'text/event-stream' });
    (options.stream ?? defaultStream)(response);
  });

  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      resolve({ origin: `http://127.0.0.1:${port}`, received,
        close: () => new Promise((done) => server.close(done)) });
    });
  });
}

function json(response, status, body) {
  const text = JSON.stringify(body);
  response.writeHead(status, { 'content-type': 'application/json' });
  response.end(text);
}

function defaultStream(response) {
  response.write('data: {"type":"response.output_text.delta","delta":"OK"}\n\n');
  response.write('data: [DONE]\n\n');
  response.end();
}
