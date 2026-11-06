// Optional interactive Framer sign-in in an owned, visible browser window.
// Adapted from T3 Code (https://github.com/pingdotgg/t3code), MIT License,
// Copyright (c) 2026 T3 Tools Inc., commit fbe5df2d; see T3-ATTRIBUTION.md.
// htmlRender/headlessChrome.ts: pipe launch, NUL framing, forced kill, temporary
// profile. cloud/CliTokenManager.ts: bounded wait raced against terminal input.
// The server runtime never imports this module.
import { spawn as nodeSpawn, type ChildProcess, type SpawnOptions } from 'node:child_process';
import { constants } from 'node:fs';
import { access, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import { delimiter, join } from 'node:path';
import type { Readable, Writable } from 'node:stream';
import { createFramerAuth, FramerAuthError, type FramerAuth } from './framer-auth.js';

export const FRAMER_LOGIN_URL = 'https://framer.com/login';
export const LOGIN_TIMEOUT_MS = 10 * 60_000;
const CLOSE_GRACE_MS = 2_000;
const KILL_WAIT_MS = 2_000;
const POLL_MS = 50;
const MAX_MESSAGE_BYTES = 1024 * 1024;
const PASTE_HINT = 'To enter an existing session instead, run accounts add ID --project-id PROJECT'
  + ' for a hidden prompt, or add --session-stdin to read it from stdin.';

export type BrowserLoginCode = 'unsupported' | 'browser_missing' | 'launch_failed' | 'cancelled'
  | 'browser_closed' | 'timeout' | 'protocol' | 'auth_refused' | 'auth_unavailable' | 'cleanup_failed';
const FAILURES: Readonly<Record<BrowserLoginCode, readonly [number, string]>> = {
  unsupported: [2, `Browser sign-in needs a terminal and a desktop session on Linux or macOS. ${PASTE_HINT}`],
  browser_missing: [2, `No Chrome or Chromium was found. Set FRAMERIZE_BROWSER to its path. ${PASTE_HINT}`],
  launch_failed: [1, `The browser could not start. ${PASTE_HINT}`],
  cancelled: [130, 'Sign-in was cancelled.'],
  browser_closed: [130, 'The browser closed before sign-in finished.'],
  timeout: [1, 'Sign-in timed out.'],
  protocol: [1, 'The browser sent an unexpected reply.'],
  auth_refused: [1, 'Framer refused this session.'],
  auth_unavailable: [1, 'Framer could not verify the session. Try again later.'],
  cleanup_failed: [1, 'The sign-in browser or its temporary framerize-login-* profile may remain. Close it and delete that directory.'],
};
export class BrowserLoginError extends Error {
  readonly exitCode: number;
  constructor(readonly code: BrowserLoginCode) {
    super(FAILURES[code][1]);
    this.name = 'BrowserLoginError';
    this.exitCode = FAILURES[code][0];
  }
}

/** Node's `child_process.spawn` signature. Tests pass a wrapper that starts a fake browser. */
export type SpawnBrowser = (command: string, args: readonly string[], options: SpawnOptions) => ChildProcess;
/** `process.kill`. Tests pass a wrapper that can refuse a signal. */
export type SignalProcess = (pid: number, signal: NodeJS.Signals | 0) => void;
export interface BrowserLoginOptions {
  /** True only when stdin and the notice stream are terminals. */
  readonly interactive: boolean;
  /** Resolves when the user reports that sign-in finished, normally on Enter. */
  readonly confirm: (signal: AbortSignal) => Promise<void>;
  /** Shows one line of guidance, normally on stderr. It never receives the session. */
  readonly notice: (text: string) => void;
  readonly signal?: AbortSignal;
  readonly timeoutMs?: number;
  readonly browser?: string;
  readonly env?: NodeJS.ProcessEnv;
  readonly platform?: NodeJS.Platform;
  readonly spawn?: SpawnBrowser;
  readonly kill?: SignalProcess;
  readonly framerOrigin?: string;
  readonly temporaryDirectory?: string;
}

const LINUX_BROWSERS = ['google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser'];
const MAC_BROWSERS = ['/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  '/Applications/Chromium.app/Contents/MacOS/Chromium'];

/** Adapted from `hasGraphicalLinuxSession` in T3's `process/externalLauncher.ts`. */
export function supportsBrowserLogin(input: Pick<BrowserLoginOptions, 'interactive' | 'env' | 'platform'>): boolean {
  const env = input.env ?? process.env;
  const platform = input.platform ?? process.platform;
  return input.interactive && (platform === 'darwin' || platform === 'linux'
    && [env.DISPLAY, env.WAYLAND_DISPLAY].some(value => value !== undefined && value.trim().length > 0));
}

/** An explicit path, then FRAMERIZE_BROWSER, then the usual Chrome and Chromium locations. */
export async function findBrowser(input: Pick<BrowserLoginOptions, 'browser' | 'env' | 'platform'>): Promise<string | undefined> {
  const env = input.env ?? process.env;
  const explicit = input.browser ?? env.FRAMERIZE_BROWSER;
  const candidates = explicit ? [explicit] : (input.platform ?? process.platform) === 'darwin' ? MAC_BROWSERS
    : (env.PATH ?? '').split(delimiter).filter(Boolean).flatMap(dir => LINUX_BROWSERS.map(name => join(dir, name)));
  for (const candidate of candidates) {
    try { await access(candidate, constants.X_OK); return candidate; } catch { /* try the next one */ }
  }
  return undefined;
}

// RFC 6265 cookie-octet: visible ASCII except quote, comma, semicolon and backslash.
const COOKIE_VALUE = /^[\x21\x23-\x2b\x2d-\x3a\x3c-\x5b\x5d-\x7e]{1,16000}$/;

/** Keeps only the `session` cookie whose domain is exactly `.framer.com` or `framer.com`. */
export function sessionFromCookies(result: unknown): string | undefined {
  const cookies = result !== null && typeof result === 'object' ? (result as { cookies?: unknown }).cookies : undefined;
  if (!Array.isArray(cookies)) throw new BrowserLoginError('protocol');
  for (const cookie of cookies) {
    if (cookie === null || typeof cookie !== 'object') continue;
    const { name, value, domain } = cookie as Record<string, unknown>;
    if (name !== 'session' || (domain !== '.framer.com' && domain !== 'framer.com')) continue;
    if (typeof value === 'string' && COOKIE_VALUE.test(value)) return `session=${value}`;
  }
  return undefined;
}

interface Pending { resolve(result: unknown): void; reject(error: Error): void }
interface DevtoolsPipe { send(method: string): Promise<unknown>; post(method: string): void }

/**
 * Browser-level DevTools commands over `--remote-debugging-pipe`: fd 3 carries
 * commands, fd 4 carries NUL-delimited JSON replies, as in T3's `launchBrowser`.
 * A reply past MAX_MESSAGE_BYTES or one that is not a JSON object fails every waiter.
 */
function devtoolsPipe(commands: Writable, replies: Readable, onFailure: (error: BrowserLoginError) => void): DevtoolsPipe {
  const pending = new Map<number, Pending>();
  let partial: Buffer[] = [];
  let partialBytes = 0;
  let failed: BrowserLoginError | undefined;
  let nextId = 0;
  const fail = (error: BrowserLoginError) => {
    if (failed) return;
    failed = error;
    for (const waiter of pending.values()) waiter.reject(error);
    pending.clear();
    partial = [];
    onFailure(error);
  };
  const receive = (bytes: Buffer) => {
    let message: unknown;
    try { message = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)); }
    catch { return fail(new BrowserLoginError('protocol')); }
    if (message === null || typeof message !== 'object' || Array.isArray(message)) {
      return fail(new BrowserLoginError('protocol'));
    }
    const { id, result, error } = message as Record<string, unknown>;
    if (id === undefined) return; // An event; no domain was enabled, so none matters.
    if (typeof id !== 'number' || !Number.isSafeInteger(id) || id < 1 || id > nextId) {
      return fail(new BrowserLoginError('protocol'));
    }
    const waiter = pending.get(id);
    if (!waiter) return; // The reply to a posted command, which nobody awaits.
    pending.delete(id);
    if (error !== undefined || result === undefined) waiter.reject(new BrowserLoginError('protocol'));
    else waiter.resolve(result);
  };
  // A chunk may end mid-message; NUL never occurs inside UTF-8 JSON text.
  const keep = (bytes: Buffer) => {
    partialBytes += bytes.length;
    if (partialBytes > MAX_MESSAGE_BYTES) fail(new BrowserLoginError('protocol'));
    else partial.push(bytes);
  };
  replies.on('data', (chunk: Buffer) => {
    let start = 0;
    for (let end = chunk.indexOf(0); end !== -1 && !failed; end = chunk.indexOf(0, start)) {
      keep(chunk.subarray(start, end));
      start = end + 1;
      if (failed) return;
      const bytes = Buffer.concat(partial);
      partial = [];
      partialBytes = 0;
      receive(bytes);
    }
    if (!failed && start < chunk.length) keep(chunk.subarray(start));
  });
  // The pipe ends when the browser exits, so an end is a closed window.
  replies.once('end', () => fail(new BrowserLoginError('browser_closed')));
  replies.on('error', () => fail(new BrowserLoginError('browser_closed')));
  commands.on('error', () => fail(new BrowserLoginError('browser_closed')));
  const write = (id: number, method: string) => {
    if (!commands.destroyed) commands.write(`${JSON.stringify({ id, method, params: {} })}\0`);
  };
  return {
    // Writes even after a failure, so a browser that sent bad input can still close itself.
    post(method) { write(++nextId, method); },
    send(method) {
      if (failed) return Promise.reject(failed);
      const id = ++nextId;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        write(id, method);
      });
    },
  };
}

interface OwnedBrowser { readonly pipe: DevtoolsPipe; close(): Promise<boolean> }

// A visible window, so no --headless. No TCP debugging port opens, so other local users cannot attach.
export function browserArguments(profile: string): string[] {
  return ['--remote-debugging-pipe', '--no-first-run', '--no-default-browser-check',
    `--user-data-dir=${profile}`, FRAMER_LOGIN_URL];
}

// The browser leads its own process group so its helpers stop with it, and a
// terminal Ctrl+C reaches this command first, which then closes it.
async function startBrowser(spawn: SpawnBrowser, signal: SignalProcess, executable: string, profile: string,
  onFailure: (error: BrowserLoginError) => void): Promise<OwnedBrowser> {
  let child: ChildProcess;
  try {
    child = spawn(executable, browserArguments(profile),
      { detached: true, stdio: ['ignore', 'ignore', 'ignore', 'pipe', 'pipe'] });
  } catch { throw new BrowserLoginError('launch_failed'); }
  let running = child.pid !== undefined;
  // A browser helper could keep fd 4 open, so the exit itself also ends the login.
  child.once('exit', () => { running = false; onFailure(new BrowserLoginError('browser_closed')); });
  child.on('error', () => onFailure(new BrowserLoginError('launch_failed')));
  const commands = child.stdio[3] as Writable | null | undefined;
  const replies = child.stdio[4] as Readable | null | undefined;
  // The group outlives its leader while any helper remains. EPERM still means it exists.
  const groupAlive = () => {
    if (child.pid === undefined) return false;
    try { signal(-child.pid, 0); return true; }
    catch (error) { return (error as NodeJS.ErrnoException).code === 'EPERM'; }
  };
  // Signals the process group this call started, never a process found by name.
  const killGroup = () => {
    if (child.pid === undefined) return;
    try { signal(-child.pid, 'SIGKILL'); } catch { /* the leader alone, below */ }
    try { if (running) signal(child.pid, 'SIGKILL'); } catch { /* close reports it */ }
  };
  // Asks the browser to close, then SIGKILLs the group after the grace period.
  // Helpers left after the leader exits are killed at once. The wait is
  // bounded, and false reports an owned process that may still run.
  const close = async (): Promise<boolean> => {
    if (running) pipe?.post('Browser.close');
    const killAt = Date.now() + (running ? CLOSE_GRACE_MS : 0);
    const giveUpAt = killAt + KILL_WAIT_MS;
    let killed = false;
    while ((running || groupAlive()) && Date.now() < giveUpAt) {
      if (!killed && (!running || Date.now() >= killAt)) { killed = true; killGroup(); }
      await delay(POLL_MS);
    }
    commands?.destroy();
    replies?.destroy();
    return !running && !groupAlive();
  };
  const pipe = commands && replies ? devtoolsPipe(commands, replies, onFailure) : undefined;
  if (!pipe) {
    const stopped = await close();
    throw new BrowserLoginError(stopped ? 'launch_failed' : 'cleanup_failed');
  }
  return { pipe, close };
}

function raced<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) return Promise.reject(signal.reason);
  return new Promise((resolve, reject) => {
    const abort = () => reject(signal.reason);
    signal.addEventListener('abort', abort, { once: true });
    promise.then(resolve, reject).finally(() => signal.removeEventListener('abort', abort));
  });
}

// Returns false while Framer still answers 401, so the user can finish signing in.
async function verified(sessionCookie: string, origin: string | undefined, signal: AbortSignal): Promise<boolean> {
  let auth: FramerAuth;
  try { auth = createFramerAuth({ sessionCookie, ...(origin ? { origin } : {}) }); }
  catch { throw new BrowserLoginError('protocol'); }
  try { await auth.accessToken({ signal }); return true; }
  catch (error) {
    signal.throwIfAborted();
    if (error instanceof FramerAuthError && error.code === 'session_unavailable') return false;
    throw new BrowserLoginError(error instanceof FramerAuthError && error.code === 'auth_refused'
      ? 'auth_refused' : 'auth_unavailable');
  } finally { await auth.close(); }
}

const START_NOTICE = 'Sign in to Framer in the opened window. Press Enter when your dashboard shows.'
  + ' Press Ctrl+C to cancel.';
const RETRY_NOTICE = 'Framer has not signed this window in yet. Finish signing in, then press Enter.';

/**
 * Opens a visible browser with a new temporary profile at Framer's sign-in
 * page, waits for the user, and resolves to `session=<value>`. The caller
 * stores it through the account store and never prints it. Rejects with
 * BrowserLoginError. Cleanup finishes within about 4 seconds before this
 * settles. If an owned process may remain or the profile could not be
 * removed, it rejects with `cleanup_failed`, even after a captured session.
 */
export async function loginWithBrowser(options: BrowserLoginOptions): Promise<string> {
  // Listen before the first await; an abort fired earlier is then rechecked below.
  const stop = new AbortController();
  const cancel = () => stop.abort(new BrowserLoginError('cancelled'));
  options.signal?.addEventListener('abort', cancel, { once: true });
  if (options.signal?.aborted) cancel();
  const timer = setTimeout(() => stop.abort(new BrowserLoginError('timeout')), options.timeoutMs ?? LOGIN_TIMEOUT_MS);
  let profile: string | undefined;
  let browser: OwnedBrowser | undefined;
  try {
    stop.signal.throwIfAborted();
    if (!supportsBrowserLogin(options)) throw new BrowserLoginError('unsupported');
    const executable = await raced(findBrowser(options), stop.signal);
    if (!executable) throw new BrowserLoginError('browser_missing');
    stop.signal.throwIfAborted();
    // mkdtemp creates the directory with mode 0700.
    try { profile = await mkdtemp(join(options.temporaryDirectory ?? tmpdir(), 'framerize-login-')); }
    catch { throw new BrowserLoginError('launch_failed'); }
    stop.signal.throwIfAborted();
    const signalProcess: SignalProcess = options.kill ?? ((pid, name) => { process.kill(pid, name); });
    browser = await startBrowser(options.spawn ?? nodeSpawn, signalProcess, executable, profile,
      error => stop.abort(error));
    stop.signal.throwIfAborted();
    options.notice(START_NOTICE);
    for (;;) {
      await raced(options.confirm(stop.signal).catch(() => { throw new BrowserLoginError('cancelled'); }), stop.signal);
      const session = sessionFromCookies(await raced(browser.pipe.send('Storage.getCookies'), stop.signal));
      if (session && await verified(session, options.framerOrigin, stop.signal)) return session;
      options.notice(RETRY_NOTICE);
    }
  } catch (error) {
    if (stop.signal.aborted && stop.signal.reason instanceof BrowserLoginError) throw stop.signal.reason;
    throw error instanceof BrowserLoginError ? error : new BrowserLoginError('protocol');
  } finally {
    clearTimeout(timer);
    options.signal?.removeEventListener('abort', cancel);
    const stopped = browser ? await browser.close() : true;
    // The profile holds every cookie from the sign-in, so a failed removal fails the login.
    let removed = true;
    if (profile) {
      try { await rm(profile, { recursive: true, force: true, maxRetries: 3 }); }
      catch { removed = false; }
    }
    // Reported even after success: the caller must not claim every process stopped.
    if (!stopped || !removed) throw new BrowserLoginError('cleanup_failed');
  }
}
