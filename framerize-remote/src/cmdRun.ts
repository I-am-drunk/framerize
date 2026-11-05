import { createInstance, type ReadinessReason } from '@framerize/core';
import { resolveInstance } from './resolveInstance.js';
import { readinessLine } from './present.js';
import { DEFAULT_BACKEND_TIMINGS } from './backendConfig.js';
import { diagnostic, diagnosticText, RemoteDiagnosticError, type DiagnosticCode } from './diagnostics.js';
import { readTailscaleStatus } from './tailscale.js';
import type { ListenOverrides } from './listenOptions.js';

export interface CommandResult {
  readonly exitCode: number;
  readonly text: string;
}

export async function runCheck(
  configPath: string,
  json: boolean,
  env?: NodeJS.ProcessEnv,
  tailscale = false,
  signal: AbortSignal = new AbortController().signal,
): Promise<CommandResult> {
  const { backend } = await resolveInstance(configPath, env);
  try {
    const { readiness: result, code } = await backend.diagnose({ signal });
    const line = readinessLine(result);
    const local = tailscale ? await readTailscaleStatus({ signal, ...(env ? { env } : {}) }) : undefined;
    const details = result === true ? { ready: true } : { ...result, diagnostic: diagnostic(code ?? result.reason) };
    const text = json
      ? JSON.stringify({ ...details, ...(local ? { tailscale: local } : {}) })
      : (code ? diagnosticText(code) : line.text) + (local
        ? `\nTailscale: ${local.message}\nLocal status does not prove remote reachability.` : '');
    return { exitCode: line.ok && !signal.aborted ? 0 : 1, text };
  } finally {
    await backend.close();
  }
}

export const SERVE_PID_HEADER = 'x-framerize-remote-pid';

export interface ServeHandle {
  readonly address: { host: string; port: number };
  close(): Promise<void>;
}

export class StartupReadinessError extends Error {
  constructor(readonly reason: ReadinessReason, readonly code: DiagnosticCode = reason) {
    super('remote is not ready');
  }
}

export class StartupCleanupError extends RemoteDiagnosticError {
  constructor() { super('cleanup_failed'); }
}

export async function startServe(
  configPath: string,
  env?: NodeJS.ProcessEnv,
  signal: AbortSignal = new AbortController().signal,
  overrides: ListenOverrides = {},
): Promise<ServeHandle> {
  const { config, localToken, backend } = await resolveInstance(configPath, env, overrides);
  let close = async (): Promise<void> => { await backend.close(); };
  try {
    signal.throwIfAborted();
    const { readiness: ready, code } = await backend.diagnose({ signal });
    signal.throwIfAborted();
    if (ready !== true) throw new StartupReadinessError(ready.reason, code);
    const instance = createInstance({
      backend, token: localToken, ...(localToken === null ? { authentication: 'none' as const } : {}),
      host: config.host, port: config.port,
      headersTimeoutMs: DEFAULT_BACKEND_TIMINGS.readinessWholeMs,
      inferenceTimeoutMs: DEFAULT_BACKEND_TIMINGS.readinessWholeMs + config.inferenceTimeoutMs,
      idleTimeoutMs: DEFAULT_BACKEND_TIMINGS.idleTimeoutMs,
    });
    close = () => instance.close();
    // Lets `frm up` tell this process from another listener on the port, as the Server's
    // x-framerize-server-pid header does. /health needs no local token.
    instance.server.prependListener('request', (request, response) => {
      if (request.url === '/health') response.setHeader(SERVE_PID_HEADER, String(process.pid));
    });
    const info = await instance.listen().catch(() => { throw new RemoteDiagnosticError('listen_failed'); });
    signal.throwIfAborted();
    return { address: { host: config.host, port: info.port }, close };
  } catch (error) {
    try { await close(); }
    catch { throw new StartupCleanupError(); }
    throw error;
  }
}
