import { open } from 'node:fs/promises';
import { constants } from 'node:fs';
import { ConfigurationError } from './errors.js';

const DEFAULTS = Object.freeze({
  host: '127.0.0.1', port: 8789, accountsFile: '', upstream: '',
  serverTokenEnv: 'FRAMERIZE_SERVER_TOKEN', serverTokenFile: '',
  authorizationEnv: '', authorizationFile: '', logLevel: 'info',
  maxBodyBytes: 16 * 1024 * 1024, headersTimeoutMs: 15_000,
  idleTimeoutMs: 60_000, requestHeadersTimeoutMs: 15_000,
  shutdownTimeoutMs: 5_000,
});

function invalid(name) {
  throw new ConfigurationError(`Invalid ${name}`);
}

export function loadConfig(values = {}, env = process.env) {
  if (!values || typeof values !== 'object' || Array.isArray(values)) invalid('configuration');
  if (Object.hasOwn(values, 'requestTimeoutMs')) throw new ConfigurationError('requestTimeoutMs is not supported');
  for (const name of Object.keys(values)) {
    if (!Object.hasOwn(DEFAULTS, name) && name !== 'serverToken') invalid('configuration option');
  }
  const config = { ...DEFAULTS, ...values };
  for (const name of ['port', 'maxBodyBytes', 'headersTimeoutMs', 'idleTimeoutMs',
    'requestHeadersTimeoutMs', 'shutdownTimeoutMs']) {
    const value = config[name];
    if (typeof value !== 'number' && (typeof value !== 'string' || !/^\d+$/.test(value))) invalid(name);
    const number = Number(value);
    const maximum = name === 'port' ? 65_535 : 2_147_483_647;
    if (!Number.isSafeInteger(number) || number < (name === 'port' ? 0 : 1) || number > maximum) invalid(name);
    config[name] = number;
  }
  if (typeof config.host !== 'string' || !config.host || /[\s\0/]/.test(config.host)) invalid('host');
  if (!['debug', 'info', 'warn', 'error', 'silent'].includes(config.logLevel)) invalid('logLevel');
  for (const name of ['accountsFile', 'serverTokenFile', 'authorizationFile']) {
    if (typeof config[name] !== 'string' || config[name].includes('\0')) invalid(name);
  }
  for (const name of ['serverTokenEnv', 'authorizationEnv']) {
    if (typeof config[name] !== 'string' || (config[name] && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(config[name]))) invalid(name);
  }
  if (config.authorizationEnv && config.authorizationFile) invalid('upstream credential references');
  if (typeof config.upstream !== 'string') invalid('upstream');
  if (config.upstream) {
    let url;
    try { url = new URL(config.upstream); } catch { invalid('upstream'); }
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.search || url.hash) invalid('upstream');
    config.upstream = url.href;
  }
  const token = values.serverToken ?? (config.serverTokenFile ? '' : env[config.serverTokenEnv] ?? '');
  if (typeof token !== 'string' || (token && !/^[A-Za-z0-9._~+/-]+=*$/.test(token))) invalid('server token');
  if (values.serverToken && config.serverTokenFile) invalid('server credential references');
  Object.defineProperty(config, 'serverToken', { value: token, enumerable: false });
  return Object.freeze(config);
}

export async function readServerToken(config) {
  if (!config.serverTokenFile) return config.serverToken;
  let file;
  try {
    file = await open(config.serverTokenFile, constants.O_RDONLY | constants.O_NONBLOCK);
    const info = await file.stat();
    if (!info.isFile() || info.size > 16_384 || (process.platform !== 'win32' && (info.mode & 0o077))) invalid('server token file');
    const buffer = Buffer.alloc(16_385);
    const { bytesRead } = await file.read(buffer, 0, buffer.length, 0);
    if (bytesRead > 16_384) invalid('server token file');
    const token = buffer.subarray(0, bytesRead).toString('utf8').trim();
    if (!token || !/^[A-Za-z0-9._~+/-]+=*$/.test(token)) invalid('server token file');
    return token;
  } catch { throw new ConfigurationError('Server token file unavailable'); }
  finally { await file?.close(); }
}
