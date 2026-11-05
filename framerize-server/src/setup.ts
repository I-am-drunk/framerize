// Setup, route selection and service reconciliation adapted from T3 Code
// apps/server/src/cli/up.ts at 5a906583fed3fa41dc806f2a19bd9d095537ed3e. MIT, T3 Tools Inc.
import { access } from 'node:fs/promises';
import { homedir, userInfo } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isSea } from 'node:sea';
import { setTimeout as delay } from 'node:timers/promises';
import { parseArgs } from './cli.js';
import { loadConfig, readServerToken } from './config.js';
import { loadAccounts } from './accounts.js';
import { captureSettings, configuredSettings, serverHome } from './settings.js';
import { buildPairingUrl, formatHostForUrl, isWildcardHost, renderTerminalQrCode,
  resolveHeadlessConnectionString } from './startup-access.js';
import { controlService, ensureServiceHost, reconcileService, serviceStatus, ServerSetupError,
  type StepRunner } from './service-control.js';
import { prepareTailscale } from './tailscale.js';
import { classifyReadiness } from '../dist/core.js';
import type { ServicePlan } from './service-units.js';

export interface SetupDependencies {
  readonly env?: NodeJS.ProcessEnv;
  readonly stdout: { write(text: string): unknown };
  readonly ask?: (question: string) => Promise<string>;
  readonly program?: readonly string[];
  readonly platform?: NodeJS.Platform;
  readonly uid?: number;
  readonly run?: StepRunner;
  readonly ready?: (origin: string, token: string) => Promise<void>;
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

async function waitForReady(origin: string, token: string): Promise<void> {
  const deadline = Date.now() + 15_000;
  do {
    try {
      const response = await fetch(new URL('/ready', origin), {
        headers: token ? { authorization: `Bearer ${token}` } : {}, signal: AbortSignal.timeout(2000) });
      const body = new Uint8Array(await response.arrayBuffer());
      if (classifyReadiness(response.status, body).state === 'ready') return;
    } catch {}
    await delay(200);
  } while (Date.now() < deadline);
  throw new ServerSetupError('Server did not become ready. Check the provider settings and server.log.');
}

export async function runSetupCommand(argv: readonly string[], dependencies: SetupDependencies): Promise<number> {
  const [command, ...args] = argv;
  const action = command === 'service' ? args.shift() : undefined;
  const { flags, tailscalePort, values } = setupArgs(args);
  const env = dependencies.env ?? process.env;
  const plan = servicePlan(env, dependencies);
  if (command === 'status' || action === 'status') {
    const state = await serviceStatus(plan, dependencies.run);
    dependencies.stdout.write(flags.json ? `${JSON.stringify(state)}\n`
      : `Server service: ${state.running ? 'running' : state.installed ? 'stopped' : 'not installed'}\n`);
    return 0;
  }
  if (action && ['start', 'stop', 'restart', 'uninstall'].includes(action)) {
    const result = await controlService(plan, action as 'start' | 'stop' | 'restart' | 'uninstall', dependencies.run);
    dependencies.stdout.write(flags.json ? `${JSON.stringify({ service: result })}\n` : `Server service: ${result}\n`);
    return 0;
  }
  return configureAndPair(command, action, flags, tailscalePort, values, plan, dependencies);
}

async function configureAndPair(command: string | undefined, action: string | undefined,
  flags: Record<string, boolean>, tailscalePort: number, values: Record<string, string>,
  plan: ServicePlan, dependencies: SetupDependencies): Promise<number> {
  if (!['up', 'pair'].includes(command ?? '') && action !== 'install') {
    throw new ServerSetupError('Use up, pair, status, or service install|start|stop|restart|uninstall.');
  }
  const path = values.config ?? join(plan.home, 'server.json');
  const exists = await access(path).then(() => true, error => {
    if (error.code === 'ENOENT') return false;
    throw error;
  });
  const explicitPath = values.config !== undefined;
  delete values.config;
  const env = dependencies.env ?? process.env;
  if (command !== 'pair' && !exists) await chooseAccess(flags, values, env, dependencies.ask);
  if (values.privateNetworkOnly !== undefined && values.host === undefined && env.FRAMERIZE_SERVER_HOST === undefined) {
    values.host = values.privateNetworkOnly === 'true' ? '0.0.0.0' : '127.0.0.1';
  }
  const loaded = await configuredSettings(exists || explicitPath ? path : undefined, values, env);
  let config = loaded.config;
  const token = await readServerToken(config);
  if (command !== 'pair' && config.backend === 'http' && !config.upstream) {
    if (!dependencies.ask) throw new ServerSetupError('Set FRAMERIZE_SERVER_UPSTREAM or pass --upstream to choose the inference API.');
    const upstream = (await dependencies.ask('Upstream inference API URL: ')).trim();
    config = loadConfig({ ...captureSettings(config, loaded.env).config, upstream }, loaded.env);
    if (!config.upstream) throw new ServerSetupError('An upstream inference API URL is required.');
  }
  if (config.port === 0) throw new ServerSetupError('Use a fixed Server port for pairing and background startup.');
  const localHost = isWildcardHost(config.host) ? (config.host === '0.0.0.0' ? '127.0.0.1' : '::1') : config.host;
  const local = `http://${formatHostForUrl(localHost)}:${config.port}`;
  const ready = dependencies.ready ?? waitForReady;
  const servePort = flags['tailscale-port'] ? tailscalePort : loaded.saved?.access?.tailscalePort ?? tailscalePort;
  const useTailscale = flags.tailscale || Boolean(loaded.saved?.access && !flags['no-tailscale']);
  const tailnet = useTailscale ? await prepareTailscale(config.port, servePort, dependencies.run, localHost) : undefined;
  const origin = tailnet?.origin ?? resolveHeadlessConnectionString(config.host, config.port,
    { privateNetworkOnly: config.privateNetworkOnly });
  if (command === 'pair') {
    await ready(local, token);
    try { await tailnet?.apply(); if (tailnet) await ready(tailnet.origin, token); }
    catch (error) { await tailnet?.rollback(); throw error; }
    printPairing(origin, token, flags.json === true, dependencies.stdout);
    return 0;
  }
  let oldRoute: Awaited<ReturnType<typeof prepareTailscale>> | undefined;
  if (flags['no-tailscale'] && loaded.saved?.access) {
    const previous = loadConfig(loaded.saved.config, loaded.saved.environment);
    const oldHost = isWildcardHost(previous.host) ? (previous.host === '0.0.0.0' ? '127.0.0.1' : '::1') : previous.host;
    oldRoute = await prepareTailscale(previous.port, loaded.saved.access.tailscalePort, dependencies.run, oldHost);
  }
  const accounts = await loadAccounts(config.accountsFile);
  const names = accounts.flatMap(account => typeof account.session === 'object' && account.session.env
    ? [account.session.env] : []);
  const settings = { ...captureSettings(config, loaded.env, names),
    ...(tailnet ? { access: { tailscalePort: servePort } } : {}) };
  await ensureServiceHost(plan, dependencies.run);
  let result: 'installed' | 'updated' | 'unchanged';
  try {
    result = await reconcileService(plan, `${JSON.stringify(settings, null, 2)}\n`, async () => {
      await ready(local, token);
      if (tailnet) { await tailnet.apply(); await ready(tailnet.origin, token); }
      await oldRoute?.remove();
    }, dependencies.run);
  } catch (error) {
    await tailnet?.rollback();
    await oldRoute?.rollback();
    throw error;
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
  stdout.write(`This machine\n  Server: ${service ?? 'ready'}\n  URL: ${origin}\n\n${qr}Next: frm '${pairingUrl}'\n`);
}
