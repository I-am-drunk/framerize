// Port preflight and PID-checked readiness follow the tested Server correction in
// framerize-server/src/setup.ts (checkPort, waitForReady) and service-control.ts (verifyReady).
import { createServer, request, type IncomingMessage } from 'node:http';
import { setTimeout as delay } from 'node:timers/promises';
import { SERVE_PID_HEADER } from './cmdRun.js';
import { RemoteDiagnosticError } from './diagnostics.js';

/** Fails when another listener holds the port; binds exclusively and closes at once. */
export async function checkLocalPort(host: string, port: number, signal?: AbortSignal): Promise<void> {
  if (signal?.aborted) throw new RemoteDiagnosticError('aborted');
  await new Promise<void>((resolve, reject) => {
    const probe = createServer().unref();
    const finish = (error?: Error | null): void => {
      signal?.removeEventListener('abort', abort);
      if (signal?.aborted) reject(new RemoteDiagnosticError('aborted'));
      else if (error) reject(error);
      else resolve();
    };
    const abort = (): void => { probe.close(() => finish()); };
    probe.once('error', (error: NodeJS.ErrnoException) => {
      finish(new RemoteDiagnosticError(error.code === 'EADDRINUSE' ? 'service_port_busy' : 'listen_failed'));
    });
    signal?.addEventListener('abort', abort, { once: true });
    try {
      probe.listen({ host, port, exclusive: true }, () => probe.close(finish));
    } catch (error) {
      signal?.removeEventListener('abort', abort);
      reject(error);
    }
  });
}

const HEALTH_LIMIT_BYTES = 4096;
const HEALTH_TIMEOUT_MS = 2_000;

/**
 * The PID a frm serve listener reports on a complete /health 200, or undefined for any other answer,
 * including a truncated, oversized or stalled body. Settles once and always releases the socket.
 */
function healthPid(host: string, port: number, signal: AbortSignal): Promise<number | undefined> {
  return new Promise((resolve) => {
    let settled = false;
    let response: IncomingMessage | undefined;
    const finish = (pid: number | undefined): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      response?.destroy();
      client.destroy();
      resolve(pid);
    };
    const timer = setTimeout(() => finish(undefined), HEALTH_TIMEOUT_MS);
    const client = request({ host, port, path: '/health', method: 'GET', agent: false, signal });
    client.once('error', () => finish(undefined));
    client.once('response', (incoming) => { response = incoming; readHealth(incoming, finish); });
    client.end();
  });
}

/** Decides only after the whole body arrives; an aborted or oversized response counts as no answer. */
function readHealth(response: IncomingMessage, finish: (pid: number | undefined) => void): void {
  const pid = Number(response.headers[SERVE_PID_HEADER]);
  const valid = response.statusCode === 200 && Number.isSafeInteger(pid) && pid > 0;
  let bytes = 0;
  response.on('data', (chunk: Buffer) => {
    bytes += chunk.length;
    if (bytes > HEALTH_LIMIT_BYTES) finish(undefined);
  });
  response.once('error', () => finish(undefined));
  response.once('aborted', () => finish(undefined));
  response.once('close', () => finish(undefined));
  response.once('end', () => finish(valid && response.complete ? pid : undefined));
}

export interface ListenerWait {
  readonly host: string;
  readonly port: number;
  /** What the service manager reports now: whether the unit runs, and its main PID. */
  readonly service: () => Promise<{ readonly running: boolean; readonly pid: number | undefined }>;
  readonly timeoutMs?: number;
  readonly signal?: AbortSignal;
}

/**
 * Waits until the local API answers /health from the service's own main PID, as the Server's
 * waitForReady and verifyReady require. Another listener on the port never counts.
 */
export async function waitForServiceListener(wait: ListenerWait): Promise<number> {
  const deadline = AbortSignal.any([AbortSignal.timeout(wait.timeoutMs ?? 15_000),
    ...(wait.signal ? [wait.signal] : [])]);
  while (!deadline.aborted) {
    const [listener, service] = await Promise.all([healthPid(wait.host, wait.port, deadline), wait.service()]);
    if (deadline.aborted) break;
    // A unit that stopped after a successful start command failed; waiting longer cannot help.
    if (!service.running) throw new RemoteDiagnosticError('service_failed', 'the service stopped after it started');
    if (listener !== undefined && listener === service.pid) return listener;
    await delay(200, undefined, { signal: deadline }).catch(() => {});
  }
  if (wait.signal?.aborted) throw new RemoteDiagnosticError('aborted');
  throw new RemoteDiagnosticError('service_not_ready');
}
