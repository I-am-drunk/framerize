import net from 'node:net';

export function mintCredential(socketPath, accountId, timeoutMs = 15_000) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath);
    let buffer = '';
    const finish = (callback, value) => {
      clearTimeout(timer);
      socket.removeAllListeners();
      socket.destroy();
      callback(value);
    };
    const fail = (error) => finish(reject, error);
    const timer = setTimeout(() => fail(new Error('credential service timeout')), timeoutMs);
    timer.unref();
    socket.setEncoding('utf8');
    socket.on('error', fail);
    socket.on('end', () => fail(new Error('credential service closed without a response')));
    socket.on('connect', () => socket.write(`${JSON.stringify({ op: 'mint', account_id: accountId })}\n`));
    socket.on('data', (chunk) => {
      buffer += chunk;
      if (buffer.length > 16 * 1024) {
        finish(reject, new Error('credential service response too large'));
        return;
      }
      const newline = buffer.indexOf('\n');
      if (newline < 0) return;
      try {
        const result = JSON.parse(buffer.slice(0, newline));
        if (!result.ok || typeof result.token !== 'string' || !result.token) {
          finish(reject, new Error('credential service could not mint a token'));
        } else finish(resolve, result.token);
      } catch { finish(reject, new Error('invalid credential service response')); }
    });
  });
}
