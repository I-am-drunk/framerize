import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from '../framerize-remote/src/cliArgs.js';
import { loadCredential, type CredentialReference } from '../framerize-remote/src/credentials.js';
import { checkInference, validateInferenceOptions } from './validation/inference.js';

const failure = (code: 'configuration' | 'credential') => ({ ok: false as const, code,
  scope: 'single-endpoint-text-inference', requestAttempts: 0, status: null, durationMs: 0 });

export async function runInferenceCheck(argv: readonly string[], env: NodeJS.ProcessEnv,
  signal?: AbortSignal) {
  let request;
  let reference: CredentialReference;
  try {
    const { options } = parseArgs(argv, { endpoint: 'string', model: 'string',
      'token-env': 'string', 'token-file': 'string', 'timeout-ms': 'string', 'send-one-request': 'boolean' });
    if (options['send-one-request'] !== true || Boolean(options['token-env']) === Boolean(options['token-file'])) {
      throw new Error();
    }
    if (options['timeout-ms'] !== undefined && !/^\d+$/.test(String(options['timeout-ms']))) throw new Error();
    request = { endpoint: options.endpoint as string, model: options.model as string,
      timeoutMs: Number(options['timeout-ms'] ?? 30000), ...(signal ? { signal } : {}) };
    validateInferenceOptions({ ...request, token: 'preflight' });
    if (options['token-env'] && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(String(options['token-env']))) throw new Error();
    reference = options['token-env'] ? { kind: 'env', name: String(options['token-env']) }
      : { kind: 'file', path: String(options['token-file']) };
  } catch { return failure('configuration'); }
  if (signal?.aborted) return checkInference({ ...request, token: 'preflight' });
  let token: string;
  try { token = await loadCredential(reference, env); }
  catch { return failure('credential'); }
  return checkInference({ ...request, token });
}

const help = `Usage: check-inference --endpoint ORIGIN --model MODEL
  (--token-env NAME | --token-file FILE) --send-one-request [--timeout-ms N]
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
