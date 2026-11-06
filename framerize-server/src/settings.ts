import { readFile } from 'node:fs/promises';
import { isAbsolute, resolve } from 'node:path';
import { homedir } from 'node:os';
import { ConfigurationError } from './errors.js';
import { ENVIRONMENT, loadConfig, type ServerConfig } from './config.js';

type Environment = Readonly<Record<string, string | undefined>>;
export interface SavedSettings {
  readonly version: 1;
  readonly access?: { readonly tailscalePort: number };
  readonly config: Readonly<Record<string, unknown>>;
  readonly environment: Readonly<Record<string, string>>;
}

export function serverHome(env: Environment = process.env): string {
  const override = env.FRAMERIZE_SERVER_HOME;
  if (override !== undefined && (!isAbsolute(override) || /[\x00-\x1f\x7f]/.test(override))) {
    throw new ConfigurationError('FRAMERIZE_SERVER_HOME must be an absolute directory path');
  }
  return resolve(override ?? resolve(env.HOME ?? env.USERPROFILE ?? homedir(), '.framerize-server'));
}

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

export async function readSettings(path: string): Promise<SavedSettings> {
  let value: unknown;
  try { value = JSON.parse(await readFile(path, 'utf8')); }
  catch { throw new ConfigurationError('Server settings are unavailable or invalid'); }
  if (!record(value) || value.version !== 1 || !record(value.config) || !record(value.environment)) {
    throw new ConfigurationError('Invalid Server settings');
  }
  for (const [name, setting] of Object.entries(value.environment)) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name) || typeof setting !== 'string') {
      throw new ConfigurationError('Invalid Server settings environment');
    }
  }
  const access = value.access;
  if (access !== undefined && (!record(access) || !Number.isInteger(access.tailscalePort)
    || Number(access.tailscalePort) < 1 || Number(access.tailscalePort) > 65535)) {
    throw new ConfigurationError('Invalid saved Tailscale port');
  }
  return { version: 1, config: value.config, environment: value.environment as Record<string, string>,
    ...(record(access) ? { access: { tailscalePort: Number(access.tailscalePort) } } : {}) };
}

export async function configuredSettings(path: string | undefined,
  values: Readonly<Record<string, unknown>>, env: Environment) {
  const saved = path ? await readSettings(path) : undefined;
  const combinedEnv = new Proxy({ ...saved?.environment, ...env }, {
    get(target, name, receiver) {
      if (typeof name === 'string') {
        const value = env[name];
        if (typeof value === 'string' || Object.hasOwn(env, name)) return value;
      }
      return Reflect.get(target, name, receiver);
    },
  });
  const defaults: Record<string, unknown> = { ...saved?.config };
  for (const [name, variable] of Object.entries(ENVIRONMENT)) {
    if (env[variable] !== undefined) delete defaults[name];
  }
  if ((Object.hasOwn(values, 'privateNetworkOnly') || env.FRAMERIZE_SERVER_LAN !== undefined)
    && !Object.hasOwn(values, 'host') && env.FRAMERIZE_SERVER_HOST === undefined) {
    delete defaults.host;
  }
  const tokenKeys = ['serverToken', 'serverTokenEnv', 'serverTokenFile'];
  if (tokenKeys.some(name => Object.hasOwn(values, name))
    || ['FRAMERIZE_SERVER_TOKEN', 'FRAMERIZE_SERVER_TOKEN_ENV', 'FRAMERIZE_SERVER_TOKEN_FILE']
      .some(name => env[name] !== undefined)) {
    for (const name of tokenKeys) delete defaults[name];
  }
  if (['authorizationEnv', 'authorizationFile'].some(name => Object.hasOwn(values, name))
    || ['FRAMERIZE_SERVER_AUTHORIZATION_ENV', 'FRAMERIZE_SERVER_AUTHORIZATION_FILE']
      .some(name => env[name] !== undefined)) {
    delete defaults.authorizationEnv;
    delete defaults.authorizationFile;
  }
  return { config: loadConfig({ ...defaults, ...values }, combinedEnv), env: combinedEnv, saved };
}

export function captureSettings(config: ServerConfig, env: Environment,
  extraEnvNames: readonly string[] = []): SavedSettings {
  const environment: Record<string, string> = {};
  for (const name of [config.serverTokenEnv, config.authorizationEnv, ...extraEnvNames]) {
    const value = env[name];
    if (name && value !== undefined) Object.defineProperty(environment, name,
      { value, enumerable: true, writable: true, configurable: true });
  }
  const values: Record<string, unknown> = { ...config };
  if (!config.serverToken && !config.serverTokenFile) {
    values.serverTokenEnv = '';
    values.serverToken = '';
  }
  for (const key of ['accountsFile', 'serverTokenFile', 'authorizationFile'] as const) {
    if (config[key]) values[key] = resolve(config[key]);
  }
  if (!config.serverTokenEnv && !config.serverTokenFile) values.serverToken = config.serverToken;
  return { version: 1, config: values, environment };
}
