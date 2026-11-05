import type { ClientCommand, ClientName } from './clients.js';

/** Local Remote address and optional local bearer token for one client session. */
export interface RemoteTarget {
  readonly origin: string;
  readonly localToken?: string;
}

export const CODEX_PROVIDER_ID = 'framerize';
export const LOCAL_TOKEN_ENV = 'FRM_LOCAL_TOKEN';

/** Core serves Claude at /anthropic/v1/messages; Claude Code appends /v1/messages. */
export function claudeBaseUrl(origin: string): string { return `${origin}/anthropic`; }
/** Core serves Codex at /openai/v1/responses; Codex appends /responses. */
export function codexBaseUrl(origin: string): string { return `${origin}/openai/v1`; }

const toml = (value: string): string => JSON.stringify(value);

/** Codex `-c` overrides. The token stays in FRM_LOCAL_TOKEN, never in arguments. */
export function codexOverrides(target: RemoteTarget): string[] {
  const fields = [`name=${toml('Framerize Remote')}`, `base_url=${toml(codexBaseUrl(target.origin))}`,
    `wire_api=${toml('responses')}`, ...(target.localToken ? [`env_key=${toml(LOCAL_TOKEN_ENV)}`] : [])];
  return ['-c', `model_providers.${CODEX_PROVIDER_ID}={${fields.join(',')}}`,
    '-c', `model_provider=${toml(CODEX_PROVIDER_ID)}`];
}

/** Builds the client launch. Caller arguments such as --model pass through unchanged. */
export function launchCommand(client: ClientName, executable: string, target: RemoteTarget,
  passthrough: readonly string[]): ClientCommand {
  if (client === 'codex') {
    const args = [...codexOverrides(target), ...passthrough];
    return { command: executable, args, display: `codex ${args.join(' ')}`,
      ...(target.localToken ? { env: { [LOCAL_TOKEN_ENV]: target.localToken } } : {}) };
  }
  return { command: executable, args: [...passthrough], display: ['claude', ...passthrough].join(' '),
    env: claudeEnv(target) };
}

/**
 * Without a credential variable Claude Code sends a saved claude.ai login to the base URL.
 * A fixed non-secret value keeps that login local when Remote runs without a local token.
 */
export const NO_LOCAL_TOKEN = 'framerize-no-local-token';

export function claudeEnv(target: RemoteTarget): Record<string, string> {
  return { ANTHROPIC_BASE_URL: claudeBaseUrl(target.origin),
    ANTHROPIC_AUTH_TOKEN: target.localToken ?? NO_LOCAL_TOKEN };
}
