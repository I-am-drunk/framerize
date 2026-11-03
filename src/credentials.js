import net from 'node:net';

export function mintCredential(socketPath, accountId, timeoutMs = 15_000) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath);
    let buffer = '';
    const fail = (error) => { socket.destroy(); reject(error); };
    const timer = setTimeout(() => fail(new Error('credential service timeout')), timeoutMs);
    timer.unref();
    socket.setEncoding('utf8');
    socket.on('error', fail);
    socket.on('connect', () => socket.write(`${JSON.stringify({ op: 'mint', account_id: accountId })}\n`));
    socket.on('data', (chunk) => {
      buffer += chunk;
      const newline = buffer.indexOf('\n');
      if (newline < 0) return;
      clearTimeout(timer);
      socket.end();
      try {
        const result = JSON.parse(buffer.slice(0, newline));
        if (!result.ok || typeof result.token !== 'string' || !result.token) {
          reject(new Error('credential service could not mint a token'));
        } else resolve(result.token);
      } catch { reject(new Error('invalid credential service response')); }
    });
  });
}
