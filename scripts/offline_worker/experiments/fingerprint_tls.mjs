// Capture the TLS ClientHello the pure-Node sender makes, locally, no external
// request. A throwaway TCP server reads the raw ClientHello bytes a Node
// client emits, then parses version, cipher suites, extensions, and ALPN.
// Reference for comparison: a real browser's ClientHello fingerprint.
import net from 'node:net';
import tls from 'node:tls';

const server = net.createServer(sock => {
  sock.once('data', buf => {
    // buf starts with the TLS record header (5 bytes) then the handshake.
    const b = buf;
    let i = 0;
    const recordType = b[i]; i += 1;           // 22 = handshake
    const recVer = b.readUInt16BE(i); i += 2;
    const recLen = b.readUInt16BE(i); i += 2;
    const hsType = b[i]; i += 1;               // 1 = client_hello
    i += 3;                                     // handshake length
    const cliVer = b.readUInt16BE(i); i += 2;
    i += 32;                                    // random
    const sidLen = b[i]; i += 1 + sidLen;
    const csLen = b.readUInt16BE(i); i += 2;
    const ciphers = [];
    for (let k = 0; k < csLen; k += 2) ciphers.push(b.readUInt16BE(i + k));
    i += csLen;
    const compLen = b[i]; i += 1 + compLen;
    const extLen = b.readUInt16BE(i); i += 2;
    const exts = [];
    let alpn = null, sigAlgs = null, groups = null;
    const extEnd = i + extLen;
    while (i + 4 <= extEnd && i + 4 <= b.length) {
      const et = b.readUInt16BE(i); const el = b.readUInt16BE(i + 2);
      exts.push(et);
      const body = b.subarray(i + 4, i + 4 + el);
      if (et === 16) { // ALPN
        const protos = [];
        let j = 2;
        while (j < body.length) { const l = body[j]; protos.push(body.subarray(j + 1, j + 1 + l).toString()); j += 1 + l; }
        alpn = protos;
      }
      if (et === 13) { const n = body.readUInt16BE(0); sigAlgs = []; for (let k = 0; k < n; k += 2) sigAlgs.push(body.readUInt16BE(2 + k)); }
      if (et === 10) { const n = body.readUInt16BE(0); groups = []; for (let k = 0; k < n; k += 2) groups.push(body.readUInt16BE(2 + k)); }
      i += 4 + el;
    }
    console.log(JSON.stringify({ recordType, recVer, cliVer, ciphers, exts, alpn, sigAlgs, groups }, null, 1));
    sock.end(); server.close(); process.exit(0);
  });
});
server.listen(0, '127.0.0.1', () => {
  const port = server.address().port;
  const c = tls.connect({ host: '127.0.0.1', port, servername: 'api.framer.com', rejectUnauthorized: false });
  c.on('error', () => {});
  c.on('secureConnect', () => c.end());
});
