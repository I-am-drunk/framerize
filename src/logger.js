const REDACTED = '[redacted]';
const SECRET_KEYS = /^(authorization|cookie|set-cookie|password|session|token|access_token|refresh_token|api[-_]?key)$/i;
const SECRET_TEXT = /(bearer\s+)[a-z0-9._~+\/-]+=*/gi;

function clean(value, depth = 0) {
  if (depth > 5) return '[truncated]';
  if (value instanceof Error) return { name: value.name, message: clean(value.message, depth + 1) };
  if (Array.isArray(value)) return value.slice(0, 100).map((item) => clean(item, depth + 1));
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [
      key,
      SECRET_KEYS.test(key) ? REDACTED : clean(item, depth + 1),
    ]));
  }
  if (typeof value === 'string') {
    const redacted = value.replace(SECRET_TEXT, `$1${REDACTED}`);
    return redacted.length > 2_000 ? `${redacted.slice(0, 2_000)}...` : redacted;
  }
  return value;
}

export function createLogger({ stream = process.stderr, level = 'info' } = {}) {
  const priorities = { debug: 10, info: 20, warn: 30, error: 40 };
  const threshold = priorities[level] ?? priorities.info;
  function write(event, fields = {}) {
    const priority = priorities[fields.level] ?? priorities.info;
    if (priority < threshold) return;
    const entry = { time: new Date().toISOString(), event, ...clean(fields) };
    stream.write(`${JSON.stringify(entry)}\n`);
  }
  return {
    debug: (event, fields) => write(event, { ...fields, level: 'debug' }),
    info: (event, fields) => write(event, { ...fields, level: 'info' }),
    warn: (event, fields) => write(event, { ...fields, level: 'warn' }),
    error: (event, fields) => write(event, { ...fields, level: 'error' }),
  };
}
