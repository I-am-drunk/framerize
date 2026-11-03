import { appError } from './transport.js';

export function lifecycle(server, config, provider, active) {
  const sockets = new Set();
  let closing = false;
  let listening;
  let closed;
  server.on('connection', (socket) => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
    if (closing) socket.destroy();
  });
  function listen() {
    if (closing) return Promise.reject(new Error('Server is closing'));
    if (listening) return listening;
    let resolve;
    let reject;
    listening = new Promise((fulfilled, failed) => { resolve = fulfilled; reject = failed; });
    const cleanup = () => { server.off('listening', ready); server.off('error', failed); };
    const ready = () => { cleanup(); resolve(server.address()); };
    const failed = (error) => { cleanup(); reject(error); };
    server.once('listening', ready);
    server.once('error', failed);
    try { server.listen(config.port, config.host); } catch (error) { failed(error); }
    return listening;
  }
  function close() {
    if (closed) return closed;
    closing = true;
    closed = Promise.resolve().then(async () => {
      for (const controller of active) controller.abort(appError(503, 'server_closing', 'Server is closing'));
      for (const socket of sockets) socket.destroy();
      await listening?.catch(() => {});
      await new Promise((resolve) => server.close(resolve));
      let timer;
      try {
        await Promise.race([Promise.resolve().then(() => provider?.close?.()), new Promise((resolve, reject) => {
          timer = setTimeout(() => reject(new Error('Provider shutdown timed out')), config.shutdownTimeoutMs);
        })]);
      } finally { clearTimeout(timer); }
    });
    return closed;
  }
  return { server, listen, close, get closing() { return closing; } };
}
