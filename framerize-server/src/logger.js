const SECRET = /authorization|cookie|password|secret|token|credential|session|api.?key|body|prompt|input|messages/i;

function redact(value, depth = 0, seen = new WeakSet()) {
  if (depth > 5) return '[truncated]';
  if (value instanceof Error) return { name: 'Error', message: '[redacted]' };
  if (value && typeof value === 'object') {
    if (seen.has(value)) return '[circular]';
    seen.add(value);
    if (Array.isArray(value)) return value.slice(0, 100).map((item) => redact(item, depth + 1, seen));
    return Object.fromEntries(Object.entries(value).slice(0, 100).map(([key, item]) => (
      [key, SECRET.test(key) ? '[redacted]' : redact(item, depth + 1, seen)]
    )));
  }
  if (typeof value === 'string') {
    return value.replace(/\b(Bearer|Basic)\s+[^\s,;"']+/gi, '$1 [redacted]')
      .replace(/(https?:\/\/)[^\s/@]+:[^\s/@]+@/gi, '$1[redacted]@').slice(0, 2000);
  }
  if (typeof value === 'bigint') return String(value);
  return value;
}

export function createLogger({ stream = process.stderr, level = 'info' } = {}) {
  const priorities = { debug: 10, info: 20, warn: 30, error: 40, silent: Infinity };
  const threshold = priorities[level] ?? priorities.info;
  function write(event, fields = {}) {
    if ((priorities[fields.level] ?? priorities.info) < threshold) return;
    try { stream.write(`${JSON.stringify({ ...redact(fields), time: new Date().toISOString(), event: redact(event) })}\n`); }
    catch {}
  }
  return Object.freeze({
    debug: (event, fields) => write(event, { ...fields, level: 'debug' }),
    info: (event, fields) => write(event, { ...fields, level: 'info' }),
    warn: (event, fields) => write(event, { ...fields, level: 'warn' }),
    error: (event, fields) => write(event, { ...fields, level: 'error' }),
  });
}
