// Try to shape Node's ClientHello to match the real browser's, using only
// stock node:tls options: Chrome's cipher list, groups, ALPN, sigalgs.
// Compare the emitted JA3 against the captured browser JA3.
import net from 'node:net';
import tls from 'node:tls';

function captureOnce(connectOpts) {
  return new Promise(resolve => {
    const server = net.createServer(sock => {
      sock.once('data', b => {
        try {
          let i = 0;
          i += 1; i += 2; i += 2; // record header
          i += 1; i += 3;         // handshake type+len
          const cliVer = b.readUInt16BE(i); i += 2;
          i += 32;
          const sidLen = b[i]; i += 1 + sidLen;
          const csLen = b.readUInt16BE(i); i += 2;
          const ciphers = [];
          for (let k = 0; k < csLen; k += 2) ciphers.push(b.readUInt16BE(i + k));
          i += csLen;
          const compLen = b[i]; i += 1 + compLen;
          const extLen = b.readUInt16BE(i); i += 2;
          const exts = []; let groups = null, pf = null, alpn = null;
          const extEnd = i + extLen;
          while (i + 4 <= extEnd && i + 4 <= b.length) {
            const et = b.readUInt16BE(i); const el = b.readUInt16BE(i + 2);
            exts.push(et);
            const body = b.subarray(i + 4, i + 4 + el);
            if (et === 10) { const n = body.readUInt16BE(0); groups = []; for (let k = 0; k < n; k += 2) groups.push(body.readUInt16BE(2 + k)); }
            if (et === 11) { const n = body[0]; pf = []; for (let k = 0; k < n; k++) pf.push(body[1 + k]); }
            if (et === 16) { const protos = []; let j = 2; while (j < body.length) { const l = body[j]; protos.push(body.subarray(j + 1, j + 1 + l).toString()); j += 1 + l; } alpn = protos; }
            i += 4 + el;
          }
          resolve({ cliVer, ciphers, exts, groups, pf, alpn });
        } catch (e) { resolve({ error: String(e.message || e) }); }
        sock.end(); server.close();
      });
      sock.on('error', () => {});
    });
    server.listen(0, '127.0.0.1', () => {
      const port = server.address().port;
      const c = tls.connect({ host: '127.0.0.1', port, servername: 'api.framer.com', rejectUnauthorized: false, ...connectOpts });
      c.on('error', () => {});
      c.on('secureConnect', () => c.end());
    });
  });
}

const shaped = await captureOnce({
  ciphers: 'TLS_AES_128_GCM_SHA256:TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256:ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-RSA-CHACHA20-POLY1305:ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-AES128-SHA:ECDHE-RSA-AES256-SHA:AES128-GCM-SHA256:AES256-GCM-SHA384:AES128-SHA:AES256-SHA',
  ecdhCurve: 'X25519:secp256r1:secp384r1',
  ALPNProtocols: ['http/1.1'],
});
console.log(JSON.stringify(shaped));
