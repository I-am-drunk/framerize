// Setup, route selection and service reconciliation adapted from T3 Code
// apps/server/src/cli/up.ts at 5a906583fed3fa41dc806f2a19bd9d095537ed3e. MIT, T3 Tools Inc.
import { access } from 'node:fs/promises';
import { createServer as createPortProbe } from 'node:net';
import { homedir, userInfo } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isSea } from 'node:sea';
import { setTimeout as delay } from 'node:timers/promises';
import { parseArgs } from './cli.js';
import { loadConfig, readServerToken, type ServerConfig } from './config.js';
import { loadAccounts } from './accounts.js';
import { captureSettings, configuredSettings, readSettings, serverHome } from './settings.js';
import { buildPairingUrl, formatHostForUrl, isWildcardHost, renderTerminalQrCode,
  resolveHeadlessConnectionString } from './startup-access.js';
import { cancellableRunner, controlService, ensureServiceHost, reconcileService, runStep,
  serviceStatus, ServerSetupError, rollbackError, writeAtomic,
  type StepRunner } from './service-control.js';
import { prepareTailscale } from './tailscale.js';
import { classifyReadiness } from '../dist/core.js';
import type { ServicePlan } from './service-units.js';

export interface SetupDependencies {
  readonly signal?: AbortSignal;
  readonly env?: NodeJS.ProcessEnv;
  readonly stdout: { write(text: string): unknown };
  readonly ask?: (question: string) => Promise<string>;
  readonly program?: readonly string[];
  readonly platform?: NodeJS.Platform;
  readonly uid?: number;
  readonly run?: StepRunner;
  readonly ready?: (origin: string, token: string, signal?: AbortSignal) => Promise<number | void>;
}

function localAddress(config: Pick<ServerConfig, 'host' | 'port'>) {
  const host = isWildcardHost(config.host) ? (config.host === '0.0.0.0' ? '127.0.0.1' : '::1') : config.host;
  return { host, origin: `http://${formatHostForUrl(host)}:${config.port}` };
}

function setupArgs(argv: readonly string[]) {
  const remaining: string[] = [];
  const flags: Record<string, boolean> = {};
  let tailscalePort = 443;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]!;
    if (['--json', '--yes', '--tailscale', '--no-tailscale', '--lan', '--no-lan'].includes(arg)) {
      flags[arg.slice(2)] = true;
    } else if (arg === '--tailscale-serve-port') {
      flags['tailscale-port'] = true;
      tailscalePort = Number(argv[++i]);
      if (!Number.isInteger(tailscalePort) || tailscalePort < 1 || tailscalePort > 65535) {
        throw new ServerSetupError('Tailscale Serve port must be between 1 and 65535.');
      }
    } else remaining.push(arg);
  }
  if (flags.lan && flags['no-lan'] || flags.tailscale && flags['no-tailscale']) {
    throw new ServerSetupError('Choose one value for each network option.');
  }
  const values: Record<string, string> = { ...parseArgs(['serve', ...remaining]).values };
  if (flags.lan || flags['no-lan']) values.privateNetworkOnly = flags.lan ? 'true' : 'false';
  return { flags, tailscalePort, values };
}

function servicePlan(env: NodeJS.ProcessEnv, dependencies: SetupDependencies): ServicePlan {
  const home = serverHome(env);
  const prefix = dependencies.program ?? (isSea() ? [process.execPath] : [process.execPath,
    fileURLToPath(new URL('../bin/framerize-server.js', import.meta.url))]);
  return { home, homeDir: env.HOME ?? env.USERPROFILE ?? homedir(),
    platform: dependencies.platform ?? process.platform,
    uid: dependencies.uid ?? userInfo().uid, xdgConfigHome: env.XDG_CONFIG_HOME,
    program: [...prefix, 'serve', '--config', join(home, 'server.json')],
    environmentPath: [dirname(prefix[0]!), '/opt/homebrew/bin', '/usr/local/bin', '/usr/bin', '/bin'].join(':') };
}

async function readReadinessBody(response: Response): Promise<Uint8Array> {
  const reader = response.body?.getReader();
  if (!reader) return new Uint8Array();
  const chunks: Uint8Array[] = [];
  let bytes = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) return Buffer.concat(chunks, bytes);
      bytes += value.byteLength;
      if (bytes > 64 * 1024) throw new ServerSetupError('Server readiness response is too large.');
      chunks.push(value);
    }
  } finally {
    try { await reader.cancel(); } finally { reader.releaseLock(); }
  }
}

async function waitForReady(origin: string, token: string, signal?: AbortSignal): Promise<number> {
  const deadline = Date.now() + 15_000;
  do {
    signal?.throwIfAborted();
    try {
      const timeout = AbortSignal.timeout(2000);
      const response = await fetch(new URL('/ready', origin), {
        headers: token ? { authorization: `Bearer ${token}` } : {},
        signal: signal ? AbortSignal.any([signal, timeout]) : timeout });
      const body = await readReadinessBody(response);
      const pid = Number(response.headers.get('x-framerize-server-pid'));
      if (classifyReadiness(response.status, body).state === 'ready'
        && Number.isSafeInteger(pid) && pid > 0) return pid;
    } catch { signal?.throwIfAborted(); }
    try { await delay(200, undefined, { signal }); }
    catch (error) { signal?.throwIfAborted(); throw error; }
  } while (Date.now() < deadline);
  throw new ServerSetupError('Server did not become ready. Check the provider settings and server.log.');
}

async function checkPort(host: string, port: number, signal?: AbortSignal): Promise<void> {
  signal?.throwIfAborted();
  await new Promise<void>((resolve, reject) => {
    const probe = createPortProbe(socket => socket.destroy()).unref();
    const finish = (error?: Error | null): void => {
      signal?.removeEventListener('abort', abort);
      if (signal?.aborted) reject(signal.reason);
      else if (error) reject(error);
      else resolve();
    };
    const abort = (): void => { probe.close(() => finish()); };
    probe.once('error', (error: NodeJS.ErrnoException) => {
      const message = error.code === 'EADDRINUSE'
        ? `The Server port ${host}:${port} is already in use. Stop that listener or choose another port.`
        : `Could not bind the Server port ${host}:${port}: ${error.code ?? error.message}`;
      finish(new ServerSetupError(message, { cause: error }));
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

export async function runSetupCommand(argv: readonly string[], dependencies: SetupDependencies): Promise<number> {
  dependencies.signal?.throwIfAborted();
  const run = cancellableRunner(dependencies.run ?? runStep, dependencies.signal);
  const [command, ...args] = argv;
  const action = command === 'service' ? args.shift() : undefined;
  const { flags, tailscalePort, values } = setupArgs(args);
  const env = dependencies.env ?? process.env;
  const plan = servicePlan(env, dependencies);
  if (command === 'status' || action === 'status') {
    const state = await serviceStatus(plan, run);
    dependencies.stdout.write(flags.json ? `${JSON.stringify(state)}\n`
      : `Server service: ${state.running ? 'running' : state.installed ? 'stopped' : 'not installed'}\n`);
    return 0;
  }
  if (action && ['start', 'stop', 'restart', 'uninstall'].includes(action)) {
    const result = action === 'uninstall' ? await uninstall(plan, run, dependencies.signal)
      : await controlService(plan, action as 'start' | 'stop' | 'restart', run);
    dependencies.stdout.write(flags.json ? `${JSON.stringify({ service: result })}\n` : `Server service: ${result}\n`);
    return 0;
  }
  return configureAndPair(command, action, flags, tailscalePort, values, plan, dependencies);
}

async function settingsExist(path: string, signal?: AbortSignal): Promise<boolean> {
  signal?.throwIfAborted();
  const pending = access(path).then(() => true, (error: NodeJS.ErrnoException) => {
    if (error.code === 'ENOENT') return false;
    throw error;
  });
  if (!signal) return pending;
  let abort = (): void => {};
  try {
    const exists = await Promise.race([pending, new Promise<never>((_resolve, reject) => {
      abort = () => reject(signal.reason);
      signal.addEventListener('abort', abort, { once: true });
      if (signal.aborted) abort();
    })]);
    signal.throwIfAborted();
    return exists;
  } catch (error) {
    signal.throwIfAborted();
    throw error;
  } finally { signal.removeEventListener('abort', abort); }
}

async function uninstall(plan: ServicePlan, run?: StepRunner, signal?: AbortSignal): Promise<'done' | 'missing'> {
  const status = await serviceStatus(plan, run);
  if (status.otherHome) throw new ServerSetupError('The Server service belongs to another FRAMERIZE_SERVER_HOME.');
  const path = join(plan.home, 'server.json');
  const exists = await settingsExist(path, signal);
  const saved = exists ? await readSettings(path, signal) : undefined;
  if (saved?.access) {
    const config = loadConfig(saved.config, saved.environment);
    const route = await prepareTailscale(config.port, saved.access.tailscalePort, run, localAddress(config).host);
    await route.remove();
  }
  const result = await controlService(plan, 'uninstall', run);
  if (saved?.access) {
    const { access: _access, ...settings } = saved;
    await writeAtomic(path, `${JSON.stringify(settings, null, 2)}\n`);
  }
  return result;
}

async function configureAndPair(command: string | undefined, action: string | undefined,
  flags: Record<string, boolean>, tailscalePort: number, values: Record<string, string>,
  plan: ServicePlan, dependencies: SetupDependencies): Promise<number> {
  const { signal } = dependencies;
  const run = cancellableRunner(dependencies.run ?? runStep, signal);
  if (!['up', 'pair'].includes(command ?? '') && action !== 'install') {
    throw new ServerSetupError('Use up, pair, status, or service install|start|stop|restart|uninstall.');
  }
  const path = values.config ?? join(plan.home, 'server.json');
  const exists = await settingsExist(path, signal);
  const explicitPath = values.config !== undefined;
  delete values.config;
  const env = dependencies.env ?? process.env;
  // configure and accounts select save server.json without an access choice; up still asks once.
  const chosen = exists && Object.hasOwn((await readSettings(path, signal)).config, 'privateNetworkOnly');
  if (command !== 'pair' && !chosen) await chooseAccess(flags, values, env, dependencies.ask);
  if (values.privateNetworkOnly !== undefined && values.host === undefined && env.FRAMERIZE_SERVER_HOST === undefined) {
    values.host = values.privateNetworkOnly === 'true' ? '0.0.0.0' : '127.0.0.1';
  }
  const loaded = await configuredSettings(exists || explicitPath ? path : undefined, values, env, signal);
  let config = loaded.config;
  const token = await readServerToken(config, signal);
  if (command !== 'pair' && config.backend === 'http' && !config.upstream) {
    if (!dependencies.ask) throw new ServerSetupError('Set FRAMERIZE_SERVER_UPSTREAM or pass --upstream to choose the inference API.');
    const upstream = (await dependencies.ask('Upstream inference API URL: ')).trim();
    config = loadConfig({ ...captureSettings(config, loaded.env).config, upstream }, loaded.env);
    if (!config.upstream) throw new ServerSetupError('An upstream inference API URL is required.');
  }
  if (config.port === 0) throw new ServerSetupError('Use a fixed Server port for pairing and background startup.');
  const { host: localHost, origin: local } = localAddress(config);
  const ready = dependencies.ready ?? waitForReady;
  const servePort = flags['tailscale-port'] ? tailscalePort : loaded.saved?.access?.tailscalePort ?? tailscalePort;
  const useTailscale = flags.tailscale || Boolean(loaded.saved?.access && !flags['no-tailscale']);
  const previous = loaded.saved?.access ? loadConfig(loaded.saved.config, loaded.saved.environment) : undefined;
  const previousLocal = previous && localAddress(previous);
  const previousTarget = loaded.saved?.access?.tailscalePort === servePort ? previousLocal?.origin : undefined;
  const tailnet = useTailscale
    ? await prepareTailscale(config.port, servePort, dependencies.run, localHost, previousTarget, signal) : undefined;
  const origin = tailnet?.origin ?? resolveHeadlessConnectionString(config.host, config.port,
    { privateNetworkOnly: config.privateNetworkOnly });
  if (command === 'pair') {
    if (tailnet && !tailnet.configured) {
      throw new ServerSetupError('Run up --tailscale first to save and start this route before pairing.');
    }
    const pid = await ready(local, token, signal);
    if (tailnet && await ready(tailnet.origin, token, signal) !== pid) {
      throw new ServerSetupError('The Tailscale route answered from another Server process.');
    }
    printPairing(origin, token, flags.json === true, dependencies.stdout);
    return 0;
  }
  let oldRoute: Awaited<ReturnType<typeof prepareTailscale>> | undefined;
  if (previous && previousLocal && loaded.saved?.access
    && (!useTailscale || loaded.saved.access.tailscalePort !== servePort)) {
    oldRoute = await prepareTailscale(previous.port, loaded.saved.access.tailscalePort,
      dependencies.run, previousLocal.host, undefined, signal);
  }
  const accounts = await loadAccounts(config.accountsFile, signal);
  const names = accounts.flatMap(account => typeof account.session === 'object' && account.session.env
    ? [account.session.env] : []);
  const settings = { ...captureSettings(config, loaded.env, names),
    ...(tailnet ? { access: { tailscalePort: servePort } } : {}) };
  await ensureServiceHost(plan, run);
  let result: 'installed' | 'updated' | 'unchanged';
  try {
    result = await reconcileService(plan, `${JSON.stringify(settings, null, 2)}\n`, async () => {
      const pid = await ready(local, token, signal);
      if (tailnet) {
        await tailnet.apply();
        const remotePid = await ready(tailnet.origin, token, signal);
        if (pid !== remotePid) throw new ServerSetupError('The Tailscale route answered from another Server process.');
      }
      await oldRoute?.remove();
      return pid;
    }, dependencies.run, () => checkPort(config.host, config.port, signal), signal);
  } catch (error) {
    const failures: unknown[] = [];
    for (const route of [tailnet, oldRoute]) {
      try { await route?.rollback(); } catch (failure) { failures.push(failure); }
    }
    throw rollbackError(error, failures);
  }
  printPairing(origin, token, flags.json === true, dependencies.stdout, result);
  return 0;
}

async function chooseAccess(flags: Record<string, boolean>, values: Record<string, string>,
  env: NodeJS.ProcessEnv, ask: SetupDependencies['ask']): Promise<void> {
  if (values.privateNetworkOnly !== undefined || env.FRAMERIZE_SERVER_LAN !== undefined || flags.tailscale) return;
  if (!flags.yes && !ask) throw new ServerSetupError('Choose access with up --lan, --tailscale, or --no-lan.');
  const answer = flags.yes ? 'lan' : (await ask!('Allow connections on LAN, Tailscale, or only this machine? [lan/tailscale/local] (lan): ')).trim();
  if (!['', 'lan', 'tailscale', 'local'].includes(answer.toLowerCase())) {
    throw new ServerSetupError('Choose lan, tailscale, or local.');
  }
  flags.tailscale = answer.toLowerCase() === 'tailscale';
  values.privateNetworkOnly = !answer || answer.toLowerCase() === 'lan' ? 'true' : 'false';
}

function printPairing(origin: string, token: string, json: boolean,
  stdout: SetupDependencies['stdout'], service?: string): void {
  const pairingUrl = buildPairingUrl(origin, token);
  if (json) {
    stdout.write(`${JSON.stringify({ origin, pairingUrl, ready: true, ...(service ? { service } : {}) })}\n`);
    return;
  }
  let qr = '';
  try { qr = `${renderTerminalQrCode(pairingUrl)}\n\n`; } catch {}
  stdout.write(`This machine\n  Server: ${service ?? 'ready'}\n  URL: ${origin}\n\n${qr}`
    + `On the other machine, with framerize-remote installed:\n  frm connect '${pairingUrl}'\n`);
}
