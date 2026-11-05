// Adapted from T3 Code apps/server/src/cloud/bootService.ts at 5a906583 (MIT, T3 Tools Inc.):
// install, restart, uninstall, status, runStep, readSystemdProblems and requireSystemdPrerequisites.
// The pinned runtime, update handoff, downgrade and memory protection are omitted.
import { execFile } from 'node:child_process';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { loadConfigFile } from './configStore.js';
import { RemoteDiagnosticError } from './diagnostics.js';
import { selectServiceManager, type ServiceHost, type ServiceManager, type ServiceStep } from './serviceManager.js';
import { renderServicePlist, renderServiceUnit, serviceHomeOf, type ServicePlan } from './serviceUnits.js';

export type StepRunner = (command: string, args: readonly string[]) => Promise<{ code: number; stdout: string }>;

/** Must exceed systemd TimeoutStopSec and launchd ExitTimeOut (both 90s) to avoid shutdown races. */
const STEP_TIMEOUT_MS = 120_000;

export const runStep: StepRunner = (command, args) => new Promise((done) => {
  execFile(command, [...args], { timeout: STEP_TIMEOUT_MS, windowsHide: true, maxBuffer: 256 * 1024 },
    (error, stdout) => done({ code: error ? (typeof error.code === 'number' ? error.code : 1) : 0, stdout: String(stdout) }));
});

/** T3's status probes use a 5 s deadline rather than the 120 s step deadline. */
const probeStep: StepRunner = (command, args) => new Promise((done) => {
  execFile(command, [...args], { timeout: 5_000, windowsHide: true, maxBuffer: 64 * 1024 },
    (error, stdout) => done({ code: error ? (typeof error.code === 'number' ? error.code : 1) : 0, stdout: String(stdout) }));
});

export interface ServiceContext {
  readonly host: ServiceHost;
  /** FRM_HOME served by the unit; the profile is `framerize-remote.json` inside it. */
  readonly home: string;
  /** The foreground command the service runs, such as `[node, bin/frm.js, 'serve']`. */
  readonly program: readonly string[];
  readonly environmentPath: string;
  readonly run?: StepRunner;
}

export interface ServiceStatus {
  readonly supported: boolean;
  readonly installed: boolean;
  readonly running: boolean;
  readonly current: boolean;
  readonly otherHome: boolean;
  readonly unitPath: string;
  readonly logPath: string;
}

export type ServiceProblem = 'user-manager-unavailable' | 'linger-unavailable' | 'linger-disabled';

/** Adapted from T3 `formatBootServiceProblem` for the problems a Linux user service can have. */
export function formatServiceProblem(problem: ServiceProblem): string {
  switch (problem) {
    case 'user-manager-unavailable':
      return 'Cannot reach the systemd user manager. Run `systemctl --user status` in a login session for the service user. Do not run frm with sudo.';
    case 'linger-unavailable':
      return 'Cannot check whether this user can run services after logout. Run `loginctl show-user "$(id -un)" --property=Linger` and check that systemd-logind is available.';
    case 'linger-disabled':
      return 'Lingering is disabled. The frm service will stop when your last login session ends and will not start at boot. Run `sudo loginctl enable-linger "$(id -un)"`, then run frm up again as your normal user.';
  }
}

const PROBLEM_CODES = { 'user-manager-unavailable': 'service_user_manager',
  'linger-unavailable': 'service_linger_unavailable', 'linger-disabled': 'service_linger' } as const;

/** Adapted from T3 `BootServicePrerequisiteError`; the code keeps the next step in CLI output. */
export class ServicePrerequisiteError extends RemoteDiagnosticError {
  constructor(readonly problem: ServiceProblem) {
    super(PROBLEM_CODES[problem], `[${problem}] ${formatServiceProblem(problem)}`);
  }
}

/** Adapted from T3 `readSystemdProblems(false)`: the user manager and lingering, with a 5 s probe each. */
async function readSystemdProblems(context: ServiceContext): Promise<ServiceProblem[]> {
  const uid = context.host.uid;
  const probe = (command: string, args: string[]) => (context.run ?? probeStep)(command, args).catch(() => undefined);
  const [manager, linger] = await Promise.all([probe('systemctl', ['--user', 'show-environment']),
    probe('loginctl', ['show-user', ...(uid === undefined ? [] : [String(uid)]), '--property=Linger', '--value'])]);
  const problems: ServiceProblem[] = [];
  if (manager?.code !== 0) problems.push('user-manager-unavailable');
  const lingering = linger?.code === 0 ? linger.stdout.trim() : '';
  if (lingering !== 'yes') problems.push(lingering === 'no' ? 'linger-disabled' : 'linger-unavailable');
  return problems;
}

/**
 * Adapted from T3 `requireSystemdPrerequisites`: fail on an unreachable user manager or unreadable
 * linger state, try the unprivileged `loginctl enable-linger`, then fail on whatever remains.
 */
export async function requireServicePrerequisites(context: ServiceContext): Promise<void> {
  if (requireManager(context).kind !== 'systemd') return;
  const problems = await readSystemdProblems(context);
  const unavailable = problems.find((problem) => problem !== 'linger-disabled');
  if (unavailable) throw new ServicePrerequisiteError(unavailable);
  if (!problems.includes('linger-disabled')) return;
  const uid = context.host.uid;
  const enabled = await (context.run ?? runStep)('loginctl',
    ['enable-linger', '--no-ask-password', ...(uid === undefined ? [] : [String(uid)])]).catch(() => undefined);
  if (enabled?.code !== 0) throw new ServicePrerequisiteError('linger-disabled');
  const remaining = await readSystemdProblems(context);
  if (remaining[0]) throw new ServicePrerequisiteError(remaining[0]);
}

function requireManager(context: ServiceContext): ServiceManager {
  const manager = selectServiceManager(context.host);
  if (!manager) throw new RemoteDiagnosticError('service_unsupported');
  return manager;
}

function servicePlan(context: ServiceContext): ServicePlan {
  return { program: context.program, home: context.home, logPath: join(context.home, 'frm-service.log') };
}

function render(manager: ServiceManager, context: ServiceContext, plan: ServicePlan): string {
  return manager.kind === 'systemd' ? renderServiceUnit(plan)
    : renderServicePlist(plan, { homeDir: context.host.homeDir, environmentPath: context.environmentPath });
}

async function runSteps(context: ServiceContext, steps: readonly ServiceStep[]): Promise<void> {
  for (const entry of steps) {
    const result = await (context.run ?? runStep)(entry.command, entry.args);
    if (result.code !== 0 && entry.optional !== true) throw new RemoteDiagnosticError('service_failed', entry.step);
  }
}

async function succeeds(context: ServiceContext, step: ServiceStep | undefined): Promise<boolean> {
  return step !== undefined && (await (context.run ?? runStep)(step.command, step.args)).code === 0;
}

async function readUnit(path: string): Promise<string | undefined> {
  try { return await readFile(path, 'utf8'); }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return undefined;
    throw new RemoteDiagnosticError('service_failed');
  }
}

const servesHome = (unit: string, home: string): boolean => {
  const installed = serviceHomeOf(unit);
  return installed !== undefined && resolve(installed) === resolve(home);
};

/** The service reads only the saved profile, never the installing shell's variables. */
async function requireServiceProfile(home: string): Promise<void> {
  const profile = await loadConfigFile(join(home, 'framerize-remote.json'));
  if (profile.serverToken?.kind === 'env' || profile.localToken?.kind === 'env') {
    throw new RemoteDiagnosticError('service_env_token');
  }
}

export async function installService(context: ServiceContext): Promise<'installed' | 'updated'> {
  const manager = requireManager(context);
  await requireServiceProfile(context.home);
  await requireServicePrerequisites(context);
  const plan = servicePlan(context);
  const unit = await readUnit(manager.unitPath);
  if (unit !== undefined && !servesHome(unit, context.home)) throw new RemoteDiagnosticError('service_other_home');
  if (unit !== undefined) await runSteps(context, manager.stop.map((step) => ({ ...step, optional: true })));
  try {
    await mkdir(dirname(manager.unitPath), { recursive: true });
    await mkdir(context.home, { recursive: true, mode: 0o700 });
    await writeFile(manager.unitPath, render(manager, context, plan), { mode: 0o600 });
  } catch { throw new RemoteDiagnosticError('service_failed', 'writing the service unit'); }
  await runSteps(context, manager.activate);
  return unit === undefined ? 'installed' : 'updated';
}

export type ServiceAction = 'start' | 'stop' | 'restart';

/** Acts only on a unit that serves this FRM_HOME; another home's service resolves `skipped`. */
export async function controlService(context: ServiceContext, action: ServiceAction): Promise<'done' | 'skipped'> {
  const manager = requireManager(context);
  const unit = await readUnit(manager.unitPath);
  if (unit === undefined || !servesHome(unit, context.home)) return 'skipped';
  await runSteps(context, manager[action]);
  return 'done';
}

export async function uninstallService(context: ServiceContext): Promise<'removed' | 'missing' | 'skipped'> {
  const manager = requireManager(context);
  const unit = await readUnit(manager.unitPath);
  if (unit === undefined) return 'missing';
  // An unreadable home stays removable; only a unit known to serve another home is kept.
  const installed = serviceHomeOf(unit);
  if (installed !== undefined && resolve(installed) !== resolve(context.home)) return 'skipped';
  await runSteps(context, manager.deactivate);
  try { await rm(manager.unitPath, { force: true }); }
  catch { throw new RemoteDiagnosticError('service_failed', 'removing the service unit'); }
  await runSteps(context, manager.finalize);
  return 'removed';
}

export async function serviceStatus(context: ServiceContext): Promise<ServiceStatus> {
  const manager = selectServiceManager(context.host);
  const logPath = servicePlan(context).logPath;
  if (!manager) return { supported: false, installed: false, running: false, current: false, otherHome: false, unitPath: '', logPath };
  const unit = await readUnit(manager.unitPath);
  const base = { supported: true, unitPath: manager.unitPath, logPath };
  if (unit === undefined) return { ...base, installed: false, running: false, current: false, otherHome: false };
  const ours = servesHome(unit, context.home);
  const [enabled, running] = await Promise.all([manager.probes.enabled === undefined || succeeds(context, manager.probes.enabled),
    succeeds(context, manager.probes.active)]);
  const current = ours && enabled && unit === render(manager, context, servicePlan(context));
  return { ...base, installed: true, running, current, otherHome: !ours };
}
