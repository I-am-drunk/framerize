import { homedir, userInfo } from 'node:os';
import { dirname } from 'node:path';
import type { CliIo } from './cli.js';
import { parseArgs } from './cliArgs.js';
import { RemoteDiagnosticError, ServiceCommandCleanupError } from './diagnostics.js';
import { profileHome } from './profilePaths.js';
import { controlService, installService, serviceStatus, uninstallService,
  type ServiceContext } from './serviceControl.js';

const HOME_SPEC = { json: 'boolean' } as const;

export function serviceContext(program: readonly string[], env: NodeJS.ProcessEnv): ServiceContext {
  const homeDir = env.HOME ?? homedir();
  let uid: number | undefined;
  try { uid = userInfo().uid; } catch {}
  const path = [dirname(program[0] ?? process.execPath), '/opt/homebrew/bin', '/usr/local/bin', '/usr/bin', '/bin'];
  return { host: { platform: process.platform, homeDir, xdgConfigHome: env.XDG_CONFIG_HOME, uid },
    home: profileHome(env, homeDir), program, environmentPath: [...new Set(path)].join(':') };
}

/** Adapted from T3 `formatServiceStatus`, without versions or memory protection. */
function formatStatus(status: Awaited<ReturnType<typeof serviceStatus>>): string {
  if (!status.supported) {
    return 'frm service\n  Status: unavailable on this machine\n  Supported on: Linux with systemd, macOS with launchd\n';
  }
  if (!status.installed) return 'frm service\n  Status: not installed\n  Next: Run `frm service install`.\n';
  const state = status.otherHome ? 'installed for another FRM_HOME'
    : `${status.current ? 'installed' : 'needs repair'} · ${status.running ? 'running' : 'stopped'}`;
  return ['frm service', `  Status: ${state}`, `  Unit: ${status.unitPath}`, `  Logs: ${status.logPath}`,
    ...(status.current || status.otherHome ? [] : ['  Next: Run `frm service install` to repair it.']), ''].join('\n');
}

const SKIPPED = 'The installed service runs another FRM_HOME, so it was not changed. Set FRM_HOME to that directory.\n';

async function serviceCommand(argv: readonly string[], io: CliIo,
  options: { program: readonly string[]; env: NodeJS.ProcessEnv; signal: AbortSignal }): Promise<number> {
  const [action, ...rest] = argv;
  const { options: flags } = parseArgs(rest, HOME_SPEC);
  const service = { ...serviceContext(options.program, options.env), signal: options.signal };
  const say = (text: string, value: Record<string, unknown>): number => {
    io.stdout(flags.json === true ? `${JSON.stringify(value)}\n` : text);
    return 0;
  };
  if (action === 'install') {
    const result = await installService(service);
    return say(`Service ${result} and started.\nLogs: ${service.home}/frm-service.log\n`, { service: result });
  }
  if (action === 'start' || action === 'stop' || action === 'restart') {
    const result = await controlService(service, action);
    const past = { start: 'Started', stop: 'Stopped', restart: 'Restarted' }[action];
    return say(result === 'done' ? `${past} the frm service.\n` : `${SKIPPED}`, { service: result });
  }
  if (action === 'uninstall') {
    const result = await uninstallService(service);
    return say(result === 'removed' ? 'Removed the frm service.\n'
      : result === 'missing' ? 'frm service is not installed.\n' : SKIPPED, { service: result });
  }
  if (action === 'status') {
    const status = await serviceStatus(service);
    return say(formatStatus(status), status as unknown as Record<string, unknown>);
  }
  io.stderr('Usage: frm service install|status|start|stop|restart|uninstall\n');
  return 2;
}

export async function runService(argv: readonly string[], io: CliIo,
  options: { program: readonly string[]; env: NodeJS.ProcessEnv }): Promise<number> {
  const controller = new AbortController();
  const stop = (): void => controller.abort();
  process.on('SIGINT', stop);
  process.on('SIGTERM', stop);
  try {
    return await serviceCommand(argv, io, { ...options, signal: controller.signal });
  } catch (error) {
    if (controller.signal.aborted && !(error instanceof ServiceCommandCleanupError)) {
      throw new RemoteDiagnosticError('aborted');
    }
    throw error;
  } finally {
    process.off('SIGINT', stop);
    process.off('SIGTERM', stop);
  }
}
