// Minimal client for the token service. One JSON line in, one JSON line out.
import net from 'node:net';
const SOCK = process.env.FRAMERIZE_TOKEN_SOCK
  || '/home/tjninja/.local/share/framerize/token.sock';
export function call(req, timeoutMs = 15000) {
  return new Promise((resolve, reject) => {
    const conn = net.createConnection(SOCK);
    let buf = '';
    const t = setTimeout(() => { conn.destroy(); reject(new Error('timeout')); }, timeoutMs);
    conn.on('connect', () => conn.write(JSON.stringify(req) + '\n'));
    conn.on('data', chunk => {
      buf += chunk;
      const i = buf.indexOf('\n');
      if (i >= 0) {
        clearTimeout(t);
        conn.end();
        try { resolve(JSON.parse(buf.slice(0, i))); }
        catch (e) { reject(e); }
      }
    });
    conn.on('error', reject);
  });
}
const op = process.argv[2] || 'ping';
const n = parseInt(process.argv[3] || '1', 10);
for (let k = 0; k < n; k++) {
  const r = await call({ op });
  if (op === 'mint') console.log(k, 'ok=' + r.ok, 'len=' + r.token_length, 'collected=' + r.collected_length, (r.error || ''));
  else if (op === 'expand') { const a = JSON.parse(Buffer.from(r.token_b64, 'base64').toString('utf8')); console.log('slots=' + a.length); }
  else console.log(JSON.stringify(r));
}
