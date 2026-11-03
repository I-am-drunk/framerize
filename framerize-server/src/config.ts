import { open, type FileHandle } from 'node:fs/promises';
import { constants } from 'node:fs';
import { ConfigurationError } from './errors.js';

export interface ServerConfig {
  readonly host: string;
  readonly port: number;
  readonly accountsFile: string;
  readonly upstream: string;
  readonly serverTokenEnv: string;
  readonly serverTokenFile: string;
  readonly serverToken: string;
  readonly authorizationEnv: string;
  readonly authorizationFile: string;
  readonly logLevel: 'debug' | 'info' | 'warn' | 'error' | 'silent';
  readonly maxBodyBytes: number;
  readonly headersTimeoutMs: number;
  readonly idleTimeoutMs: number;
  readonly requestHeadersTimeoutMs: number;
  readonly shutdownTimeoutMs: number;
}

const DEFAULTS = Object.freeze({
  host: '127.0.0.1', port: 8789, accountsFile: '', upstream: '',
  serverTokenEnv: 'FRAMERIZE_SERVER_TOKEN', serverTokenFile: '',
  authorizationEnv: '', authorizationFile: '', logLevel: 'info',
  maxBodyBytes: 16 * 1024 * 1024, headersTimeoutMs: 15_000,
  idleTimeoutMs: 60_000, requestHeadersTimeoutMs: 15_000, shutdownTimeoutMs: 5_000,
} satisfies Omit<ServerConfig, 'serverToken'>);
const ENVIRONMENT = Object.freeze({
  host: 'FRAMERIZE_SERVER_HOST', port: 'FRAMERIZE_SERVER_PORT', upstream: 'FRAMERIZE_SERVER_UPSTREAM',
});

function invalid(name: string): never {
  throw new ConfigurationError(`Invalid ${name}`);
}

export function loadConfig(values: unknown = {}, env: Readonly<Record<string, string | undefined>> = process.env): ServerConfig {
  if (!values || typeof values !== 'object' || Array.isArray(values)) invalid('configuration');
  const input = values as Record<string, unknown>;
  if (Object.hasOwn(input, 'requestTimeoutMs')) throw new ConfigurationError('requestTimeoutMs is not supported');
  for (const name of Object.keys(input)) {
    if (!Object.hasOwn(DEFAULTS, name) && name !== 'serverToken') invalid('configuration option');
  }
  const selected: Record<string, unknown> = {};
  for (const [name, variable] of Object.entries(ENVIRONMENT)) {
    if (!Object.hasOwn(input, name) && env[variable] !== undefined) selected[name] = env[variable];
  }
  const label = (name: string) => Object.hasOwn(selected, name) ? ENVIRONMENT[name as keyof typeof ENVIRONMENT] : name;
  const config: Record<string, unknown> = { ...DEFAULTS, ...selected, ...input };
  for (const name of ['port', 'maxBodyBytes', 'headersTimeoutMs', 'idleTimeoutMs',
    'requestHeadersTimeoutMs', 'shutdownTimeoutMs']) {
    const value = config[name];
    if (typeof value !== 'number' && (typeof value !== 'string' || !/^\d+$/.test(value))) invalid(label(name));
    const number = Number(value);
    const maximum = name === 'port' ? 65_535 : 2_147_483_647;
    if (!Number.isSafeInteger(number) || number < (name === 'port' ? 0 : 1) || number > maximum) invalid(label(name));
    config[name] = number;
  }
  if (typeof config.host !== 'string' || !config.host || /[\s\0/]/.test(config.host)) invalid(label('host'));
  if (typeof config.logLevel !== 'string' || !['debug', 'info', 'warn', 'error', 'silent'].includes(config.logLevel)) invalid('logLevel');
  for (const name of ['accountsFile', 'serverTokenFile', 'authorizationFile']) {
    const value = config[name];
    if (typeof value !== 'string' || value.includes('\0')) invalid(name);
  }
  for (const name of ['serverTokenEnv', 'authorizationEnv']) {
    const value = config[name];
    if (typeof value !== 'string' || (value && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(value))) invalid(name);
  }
  if (config.authorizationEnv && config.authorizationFile) invalid('upstream credential references');
  if (typeof config.upstream !== 'string') invalid(label('upstream'));
  if (config.upstream) {
    let url: URL;
    try { url = new URL(config.upstream); } catch { invalid(label('upstream')); }
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.search || url.hash) invalid(label('upstream'));
    config.upstream = url.href;
  }
  const token = input.serverToken ?? (config.serverTokenFile ? '' : env[config.serverTokenEnv as string] ?? '');
  if (typeof token !== 'string' || (token && !/^[A-Za-z0-9._~+/-]+=*$/.test(token))) invalid('server token');
  if (input.serverToken && config.serverTokenFile) invalid('server credential references');
  Object.defineProperty(config, 'serverToken', { value: token, enumerable: false });
  return Object.freeze(config) as unknown as ServerConfig;
}

export async function readServerToken(config: ServerConfig): Promise<string> {
  if (!config.serverTokenFile) return config.serverToken;
  let file: FileHandle | undefined;
  try {
    file = await open(config.serverTokenFile, constants.O_RDONLY | constants.O_NONBLOCK);
    const info = await file.stat();
    if (!info.isFile() || info.size > 16_384 || (process.platform !== 'win32' && (info.mode & 0o077))) invalid('server token file');
    const buffer = Buffer.alloc(16_385);
    let length = 0;
    while (length < buffer.length) {
      const { bytesRead } = await file.read(buffer, length, buffer.length - length, null);
      if (!bytesRead) break;
      length += bytesRead;
    }
    if (length > 16_384) invalid('server token file');
    const token = buffer.subarray(0, length).toString('utf8').trim();
    if (!token || !/^[A-Za-z0-9._~+/-]+=*$/.test(token)) invalid('server token file');
    return token;
  } catch { throw new ConfigurationError('Server token file unavailable'); }
  finally { await file?.close(); }
}
