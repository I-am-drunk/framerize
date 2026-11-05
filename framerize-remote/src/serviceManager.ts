// Adapted from T3 Code apps/server/src/cloud/bootService.ts at 5a906583 (MIT, T3 Tools Inc.):
// systemdManager, launchdManager and selectBootServiceManager, without the pinned runtime.
import { join, isAbsolute } from 'node:path';
import { SERVICE_LAUNCHD_LABEL, SERVICE_PLIST_FILE, SERVICE_UNIT_FILE } from './serviceUnits.js';

export interface ServiceStep {
  readonly step: string;
  readonly command: string;
  readonly args: readonly string[];
  /** Non-zero exit is ignored. */
  readonly optional?: boolean;
}

export interface ServiceManager {
  readonly kind: 'systemd' | 'launchd';
  readonly unitPath: string;
  readonly activate: readonly ServiceStep[];
  readonly start: readonly ServiceStep[];
  readonly stop: readonly ServiceStep[];
  readonly restart: readonly ServiceStep[];
  readonly deactivate: readonly ServiceStep[];
  readonly finalize: readonly ServiceStep[];
  readonly probes: { readonly enabled?: ServiceStep; readonly active: ServiceStep };
}

const systemctl = (step: string, ...args: string[]): ServiceStep =>
  ({ step, command: 'systemctl', args: ['--user', ...args] });

function systemdManager(unitDir: string): ServiceManager {
  const reload = systemctl('reloading systemd user units', 'daemon-reload');
  return {
    kind: 'systemd', unitPath: join(unitDir, SERVICE_UNIT_FILE),
    activate: [reload, systemctl('enabling the service', 'enable', SERVICE_UNIT_FILE),
      systemctl('starting the service', 'restart', SERVICE_UNIT_FILE)],
    start: [systemctl('starting the service', 'start', SERVICE_UNIT_FILE)],
    stop: [systemctl('stopping the service', 'stop', SERVICE_UNIT_FILE)],
    restart: [systemctl('restarting the service', 'restart', SERVICE_UNIT_FILE)],
    deactivate: [systemctl('stopping the service', 'disable', '--now', SERVICE_UNIT_FILE)],
    finalize: [reload],
    probes: { enabled: systemctl('checking the service', 'is-enabled', SERVICE_UNIT_FILE),
      active: systemctl('checking the service', 'is-active', SERVICE_UNIT_FILE) },
  };
}

// Unloaded jobs may reject bootout/enable. Bootstrap starts RunAtLoad/KeepAlive jobs.
function launchdManager(homeDir: string, uid: number): ServiceManager {
  const unitPath = join(homeDir, 'Library', 'LaunchAgents', SERVICE_PLIST_FILE);
  const domain = `gui/${uid}`;
  const target = `${domain}/${SERVICE_LAUNCHD_LABEL}`;
  const bootout: ServiceStep = { step: 'stopping the launch agent', command: 'launchctl',
    args: ['bootout', '--wait', target], optional: true };
  const bootstrap: ServiceStep = { step: 'starting the service', command: 'launchctl', args: ['bootstrap', domain, unitPath] };
  return {
    kind: 'launchd', unitPath,
    activate: [{ step: 'enabling the launch agent', command: 'launchctl', args: ['enable', target], optional: true },
      bootstrap],
    start: [bootstrap], stop: [bootout],
    restart: [{ step: 'restarting the service', command: 'launchctl', args: ['kickstart', '-k', target] }],
    deactivate: [bootout], finalize: [],
    probes: { active: { step: 'checking the service', command: 'launchctl', args: ['print', target] } },
  };
}

export interface ServiceHost {
  readonly platform: NodeJS.Platform;
  readonly homeDir: string;
  readonly xdgConfigHome?: string | undefined;
  readonly uid?: number | undefined;
}

/** Adapted from `selectBootServiceManager`. The busctl UnitPath lookup is omitted. */
export function selectServiceManager(host: ServiceHost): ServiceManager | undefined {
  if (host.homeDir === '') return undefined;
  if (host.platform === 'linux') {
    const config = host.xdgConfigHome && isAbsolute(host.xdgConfigHome) ? host.xdgConfigHome : join(host.homeDir, '.config');
    return systemdManager(join(config, 'systemd', 'user'));
  }
  if (host.platform === 'darwin' && host.uid !== undefined) return launchdManager(host.homeDir, host.uid);
  return undefined;
}
