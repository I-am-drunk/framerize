import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from '../framerize-remote/src/cliArgs.js';
import { loadCredential, type CredentialReference } from '../framerize-remote/src/credentials.js';
import { checkInference, validateInferenceOptions, type InferenceCheckOptions } from './validation/inference.js';

const failure = (code: 'configuration' | 'credential') => ({ ok: false as const, code,
  scope: 'single-endpoint-text-inference', requestAttempts: 0, status: null, durationMs: 0 });

export async function runInferenceCheck(argv: readonly string[], env: NodeJS.ProcessEnv,
  signal?: AbortSignal) {
  let request;
  let reference: CredentialReference | undefined;
  try {
    const { options } = parseArgs(argv, { endpoint: 'string', model: 'string',
      'token-env': 'string', 'token-file': 'string', 'timeout-ms': 'string',
      protocol: 'string', 'send-one-request': 'boolean' });
    if (options['send-one-request'] !== true
      || (options['token-env'] !== undefined && options['token-file'] !== undefined)) {
      throw new Error();
    }
    if (options['timeout-ms'] !== undefined && !/^\d+$/.test(String(options['timeout-ms']))) throw new Error();
    request = { endpoint: options.endpoint as string, model: options.model as string,
      timeoutMs: Number(options['timeout-ms'] ?? 30000),
      protocol: (options.protocol ?? 'chat') as NonNullable<InferenceCheckOptions['protocol']>,
      ...(signal ? { signal } : {}) };
    validateInferenceOptions(request);
    if (options['token-env'] && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(String(options['token-env']))) throw new Error();
    reference = options['token-env'] ? { kind: 'env', name: String(options['token-env']) }
      : options['token-file'] ? { kind: 'file', path: String(options['token-file']) } : undefined;
  } catch { return failure('configuration'); }
  if (signal?.aborted) return checkInference(request);
  let token: string | null = null;
  try { if (reference) token = await loadCredential(reference, env); }
  catch { return failure('credential'); }
  return checkInference({ ...request, token });
}

const help = `Usage: check-inference --endpoint ORIGIN --model MODEL
  [--token-env NAME | --token-file FILE] --send-one-request [--timeout-ms N]
  [--protocol chat|responses|messages]
The default chat check streams. Responses and Messages request completed JSON.
Omit token options for an endpoint without authentication.
Sends one text inference request. Provider usage may be charged. No retries.
Reports counts and fixed errors only, never credentials or response text.
This checks the selected endpoint, not provider identity or release readiness.
`;

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const controller = new AbortController();
  const abort = () => controller.abort();
  process.on('SIGINT', abort); process.on('SIGTERM', abort);
  try {
    if (process.argv.length === 3 && process.argv[2] === '--help') process.stdout.write(help);
    else {
      const result = await runInferenceCheck(process.argv.slice(2), process.env, controller.signal);
      console.log(JSON.stringify(result));
      process.exitCode = result.ok ? 0 : 1;
    }
  } catch {
    console.log(JSON.stringify(failure('configuration')));
    process.exitCode = 1;
  } finally {
    process.off('SIGINT', abort); process.off('SIGTERM', abort);
  }
}
