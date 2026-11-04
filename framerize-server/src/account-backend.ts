import { loadFramerAccount, readAccountSession } from './accounts.js';
import { createFramerAuth, type FramerAuth } from './framer-auth.js';
import { createFramerBackend, type FramerBackendOptions, type FramerMinter } from './framer-backend.js';
import { AccountFileError, AccountSelectionError, ConfigurationError, ProviderError } from './errors.js';
import type { InferenceBackend } from '../dist/core.js';

export interface AccountMinterContext {
  readonly accountId: string;
  readonly signal: AbortSignal;
}

export interface AccountBackendOptions extends Pick<FramerBackendOptions,
  'origin' | 'maxBodyBytes' | 'headersTimeoutMs' | 'inferenceTimeoutMs' | 'idleTimeoutMs'> {
  readonly accountsFile: string;
  readonly accountId?: string;
  readonly env?: Readonly<Record<string, string | undefined>>;
  readonly signal?: AbortSignal;
  readonly createMinter: (context: AccountMinterContext) => FramerMinter | Promise<FramerMinter>;
}

function checkSignal(signal: AbortSignal): void {
  if (Reflect.get(AbortSignal.prototype, 'aborted', signal)) throw new ProviderError('Framer account startup cancelled', {
    code: 'request_cancelled', status: 499,
  });
}

function options(input: AccountBackendOptions) {
  try {
    const { accountsFile, accountId, createMinter, env = process.env,
      signal = new AbortController().signal, origin = 'https://api.framer.com',
      maxBodyBytes = 16 * 1024 * 1024, headersTimeoutMs = 15_000,
      inferenceTimeoutMs = 300_000, idleTimeoutMs = 60_000 } = input;
    if (typeof accountsFile !== 'string' || !accountsFile || /[\x00-\x1f\x7f]/.test(accountsFile)
      || typeof createMinter !== 'function' || !env || typeof env !== 'object' || Array.isArray(env)
      || !(signal instanceof AbortSignal) || typeof origin !== 'string') throw new Error();
    const url = new URL(origin);
    if (url.username || url.password || url.search || url.hash || url.pathname !== '/'
      || url.protocol !== 'https:' && !(url.protocol === 'http:'
        && ['127.0.0.1', '[::1]'].includes(url.hostname))) throw new Error();
    for (const value of [maxBodyBytes, headersTimeoutMs, inferenceTimeoutMs, idleTimeoutMs]) {
      if (!Number.isSafeInteger(value) || value < 1 || value > 2_147_483_647) throw new Error();
    }
    Reflect.get(AbortSignal.prototype, 'aborted', signal);
    return { accountsFile, accountId, createMinter, env, signal,
      provider: { origin: url.href, maxBodyBytes, headersTimeoutMs, inferenceTimeoutMs, idleTimeoutMs } };
  } catch { throw new ConfigurationError('Invalid Framer account configuration'); }
}

export async function createAccountBackend(input: AccountBackendOptions): Promise<InferenceBackend> {
  const config = options(input);
  let auth: FramerAuth | undefined;
  let castle: FramerMinter | undefined;
  try {
    checkSignal(config.signal);
    const account = await loadFramerAccount(config.accountsFile, config.accountId);
    checkSignal(config.signal);
    const sessionCookie = await readAccountSession(account.session, config.env, config.signal);
    checkSignal(config.signal);
    auth = createFramerAuth({ sessionCookie, origin: config.provider.origin,
      timeoutMs: config.provider.headersTimeoutMs });
    castle = await config.createMinter({ accountId: account.id, signal: config.signal });
    checkSignal(config.signal);
    return createFramerBackend({ ...config.provider, projectId: account.projectId, auth, castle });
  } catch (error) {
    await Promise.allSettled([
      Promise.resolve().then(() => auth?.close()), Promise.resolve().then(() => castle?.close()),
    ]);
    checkSignal(config.signal);
    if (error instanceof AccountFileError) throw new AccountFileError('Framer account setup failed');
    if (error instanceof AccountSelectionError) throw new AccountSelectionError('Framer account setup failed');
    throw new ConfigurationError('Framer account setup failed');
  }
}
