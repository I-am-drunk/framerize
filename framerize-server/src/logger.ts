export type LogLevel = 'debug' | 'info' | 'warn' | 'error' | 'silent';

export interface LogOutput {
  write(text: string): unknown;
}

export interface Logger {
  debug(event: string, fields?: object): void;
  info(event: string, fields?: object): void;
  warn(event: string, fields?: object): void;
  error(event: string, fields?: object): void;
}

export interface LoggerOptions {
  readonly stream?: LogOutput;
  readonly level?: LogLevel;
}

function boundedValue(value: unknown, depth = 0, seen = new WeakSet<object>()): unknown {
  if (depth > 5) return '[truncated]';
  if (value && typeof value === 'object') {
    if (seen.has(value)) return '[circular]';
    seen.add(value);
    if (Array.isArray(value)) return value.slice(0, 100).map((item) => boundedValue(item, depth + 1, seen));
    const entries: [string, unknown][] = value instanceof Error
      ? [['name', value.name], ['message', value.message], ...Object.entries(value)]
      : Object.entries(value);
    return Object.fromEntries(entries.slice(0, 100).map(([key, item]) => (
      [key, boundedValue(item, depth + 1, seen)]
    )));
  }
  if (typeof value === 'string') {
    return value.slice(0, 2000);
  }
  if (typeof value === 'bigint') return String(value);
  return value;
}

export function createLogger({ stream = process.stderr, level = 'info' }: LoggerOptions = {}): Logger {
  const priorities = { debug: 10, info: 20, warn: 30, error: 40, silent: Infinity };
  const threshold = priorities[level] ?? priorities.info;
  function write(event: string, fields: object & { level: LogLevel }) {
    if ((priorities[fields.level] ?? priorities.info) < threshold) return;
    try { stream.write(`${JSON.stringify({ ...boundedValue(fields) as object, time: new Date().toISOString(), event: boundedValue(event) })}\n`); }
    catch {}
  }
  return Object.freeze<Logger>({
    debug: (event, fields) => write(event, { ...fields, level: 'debug' }),
    info: (event, fields) => write(event, { ...fields, level: 'info' }),
    warn: (event, fields) => write(event, { ...fields, level: 'warn' }),
    error: (event, fields) => write(event, { ...fields, level: 'error' }),
  });
}
