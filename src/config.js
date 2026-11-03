import { readFile, realpath, stat } from 'node:fs/promises';

const DEFAULTS = Object.freeze({
  host: '127.0.0.1',
  port: 8788,
  upstream: '',
  accounts: '',
  requestTimeoutMs: 120_000,
  idleTimeoutMs: 60_000,
  maxBodyBytes: 16 * 1024 * 1024,
});

export class ConfigError extends Error {
  constructor(message) {
    super(message);
    this.name = 'ConfigError';
  }
}
function positiveInteger(value, name) {
  const number = Number(value);
  const minimum = name === 'port' ? 0 : 1;
  if (!Number.isSafeInteger(number) || number < minimum) {
    throw new ConfigError(`${name} must be a positive integer`);
  }
  if (name.endsWith('Ms') && number > 2_147_483_647) {
    throw new ConfigError(`${name} exceeds the supported timer range`);
  }
  return number;
}

function normalizeUpstream(value, name) {
  let url;
  try {
    url = new URL(value);
  } catch {
    throw new ConfigError(`${name} must be a valid URL`);
  }
  if (!['http:', 'https:'].includes(url.protocol)) {
    throw new ConfigError(`${name} must use HTTP or HTTPS`);
  }
  if (url.username || url.password) {
    throw new ConfigError(`${name} must not contain credentials`);
  }
  return url.href.replace(/\/+$/, '');
}

export function parseConfig(values = {}) {
  const config = { ...DEFAULTS, ...values };
  config.port = positiveInteger(config.port, 'port');
  config.requestTimeoutMs = positiveInteger(config.requestTimeoutMs, 'requestTimeoutMs');
  config.idleTimeoutMs = positiveInteger(config.idleTimeoutMs, 'idleTimeoutMs');
  config.maxBodyBytes = positiveInteger(config.maxBodyBytes, 'maxBodyBytes');
  if (config.port > 65_535) throw new ConfigError('port must be at most 65535');
  if (config.upstream) {
    config.upstream = normalizeUpstream(config.upstream, 'upstream');
  }
  return Object.freeze(config);
}

export async function loadAccounts(file) {
  if (!file) return [];
  let parsed;
  try {
    parsed = JSON.parse(await readFile(file, 'utf8'));
  } catch (error) {
    throw new ConfigError(`cannot read accounts JSON: ${error.message}`);
  }
  if (!Array.isArray(parsed)) throw new ConfigError('accounts JSON must be an array');
  const ids = new Set();
  for (const [index, account] of parsed.entries()) {
    if (!account || typeof account !== 'object' || Array.isArray(account)) {
      throw new ConfigError(`accounts[${index}] must be an object`);
    }
    const allowed = new Set(['id', 'enabled', 'model', 'upstream', 'authorizationEnv',
      'provider', 'projectId', 'sessionEnv', 'castleEnv', 'authorizationFile',
      'sessionFile', 'castleFile']);
    for (const key of Object.keys(account)) {
      if (!allowed.has(key)) throw new ConfigError(`accounts[${index}] has unknown field: ${key}`);
    }
    if (account.authorizationEnv !== undefined && !/^[A-Z_][A-Z0-9_]*$/.test(account.authorizationEnv)) {
      throw new ConfigError(`accounts[${index}].authorizationEnv must be an environment variable name`);
    }
    if (account.model !== undefined && typeof account.model !== 'string') {
      throw new ConfigError(`accounts[${index}].model must be a string`);
    }
    if (account.provider !== undefined && account.provider !== 'framer') {
      throw new ConfigError(`accounts[${index}].provider must be framer`);
    }
    for (const key of ['sessionEnv', 'castleEnv']) {
      if (account[key] !== undefined && !/^[A-Z_][A-Z0-9_]*$/.test(account[key])) {
        throw new ConfigError(`accounts[${index}].${key} must be an environment variable name`);
      }
    }
    for (const key of ['authorizationFile', 'sessionFile', 'castleFile']) {
      if (account[key] !== undefined && (typeof account[key] !== 'string' || !account[key])) {
        throw new ConfigError(`accounts[${index}].${key} must be a file path`);
      }
    }
    if (account.provider === 'framer') {
      if (typeof account.projectId !== 'string' || !account.projectId) {
        throw new ConfigError(`accounts[${index}].projectId is required for Framer`);
      }
      if (!account.sessionEnv && !account.authorizationEnv
          && !account.sessionFile && !account.authorizationFile) {
        throw new ConfigError(`accounts[${index}] needs a session or authorization reference`);
      }
    }
    if (typeof account.id !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$/.test(account.id)) {
      throw new ConfigError(`accounts[${index}].id must be a short safe string`);
    }
    if (ids.has(account.id)) throw new ConfigError(`duplicate account id: ${account.id}`);
    ids.add(account.id);
    if (account.upstream !== undefined) {
      account.upstream = normalizeUpstream(account.upstream, `accounts[${index}].upstream`);
    }
    if (account.enabled !== undefined && typeof account.enabled !== 'boolean') {
      throw new ConfigError(`accounts[${index}].enabled must be boolean`);
    }
    for (const forbidden of ['token', 'session', 'cookie', 'cookies', 'password', 'authorization']) {
      if (forbidden in account) throw new ConfigError(`accounts[${index}] must not contain ${forbidden}`);
    }
  }
  return parsed.filter((account) => account.enabled !== false).map((account) => (
    Object.fromEntries(['id', 'enabled', 'model', 'upstream', 'authorizationEnv',
      'provider', 'projectId', 'sessionEnv', 'castleEnv', 'authorizationFile',
      'sessionFile', 'castleFile']
      .filter((key) => account[key] !== undefined).map((key) => [key, account[key]]))
  ));
}

export async function readCredential(file) {
  if (!file) return '';
  const resolved = await realpath(file);
  const info = await stat(resolved);
  if (!info.isFile() || info.size > 16 * 1024) {
    throw new ConfigError('credential reference must be a small regular file');
  }
  return (await readFile(resolved, 'utf8')).trim();
}

