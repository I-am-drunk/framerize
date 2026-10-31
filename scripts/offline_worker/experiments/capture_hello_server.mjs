// Local TLS ClientHello capture server. Listens on a fixed port, reads the
// raw ClientHello from whatever connects, parses the JA3-relevant fields,
// prints one JSON line, and exits. No external request is made.
import net from 'node:net';
const PORT = parseInt(process.argv[2] || '8443', 10);
const server = net.createServer(sock => {
  sock.once('data', b => {
    try {
      let i = 0;
      const recordType = b[i]; i += 1;
      i += 2; // record version
      i += 2; // record length
      const hsType = b[i]; i += 1;
      i += 3; // handshake length
      const cliVer = b.readUInt16BE(i); i += 2;
      i += 32; // random
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
      console.log(JSON.stringify({ recordType, hsType, cliVer, ciphers, exts, groups, pf, alpn }));
    } catch (e) { console.log(JSON.stringify({ error: String(e && e.message || e) })); }
    sock.end(); server.close(); process.exit(0);
  });
  sock.on('error', () => {});
});
server.listen(PORT, '127.0.0.1', () => console.error('listening on ' + PORT));
