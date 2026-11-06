// Adapted from T3 Code apps/server/src/cloud/bootService.ts at
// 5a906583fed3fa41dc806f2a19bd9d095537ed3e. MIT, T3 Tools Inc.
import { isAbsolute, join } from 'node:path';

export const SERVICE_UNIT = 'framerize-server.service';
export const SERVICE_LABEL = 'dev.framerize.server';
export interface ServicePlan {
  readonly home: string;
  readonly homeDir: string;
  readonly program: readonly string[];
  readonly environmentPath: string;
  readonly platform: NodeJS.Platform;
  readonly uid: number;
  readonly xdgConfigHome?: string | undefined;
}
export interface ServiceStep {
  readonly command: string;
  readonly args: readonly string[];
  readonly optional?: boolean;
}
export interface ServiceManager {
  readonly path: string;
  readonly contents: string;
  readonly activate: readonly ServiceStep[];
  readonly stop: readonly ServiceStep[];
  readonly restart: readonly ServiceStep[];
  readonly remove: readonly ServiceStep[];
  readonly reload: readonly ServiceStep[];
  readonly enable: readonly ServiceStep[];
  readonly active: ServiceStep;
  readonly enabled?: ServiceStep;
}

function quoteSystemdValue(value: string, command = false): string {
  const specifiers = value.replaceAll('%', '%%');
  const escaped = command ? specifiers.replaceAll('$', '$$') : specifiers;
  return /[\s"'\\]/.test(escaped)
    ? `"${escaped.replaceAll('\\', '\\\\').replaceAll('"', '\\"')}"` : escaped;
}
const xml = (value: string) => value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
const fromXml = (value: string) => value.replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&amp;', '&');

export function serviceHomeOf(contents: string): string | undefined {
  const value = /^Environment=FRAMERIZE_SERVER_HOME=(.*)$/m.exec(contents)?.[1];
  if (value !== undefined) return (value.startsWith('"') ? value.slice(1, -1)
    .replaceAll('\\"', '"').replaceAll('\\\\', '\\') : value).replaceAll('%%', '%');
  const plist = /<key>FRAMERIZE_SERVER_HOME<\/key>\s*<string>([^<]*)<\/string>/.exec(contents)?.[1];
  return plist === undefined ? undefined : fromXml(plist);
}

export function renderServiceUnit(plan: ServicePlan): string {
  const logPath = join(plan.home, 'server.log').replaceAll('%', '%%');
  return ['[Unit]', 'Description=Framerize Server', 'StartLimitIntervalSec=300', 'StartLimitBurst=5', '',
    '[Service]', 'Type=simple', 'WorkingDirectory=%h',
    `Environment=FRAMERIZE_SERVER_HOME=${quoteSystemdValue(plan.home)}`,
    `ExecStart=${plan.program.map(value => quoteSystemdValue(value, true)).join(' ')}`, 'Restart=always', 'RestartSec=5',
    'TimeoutStopSec=90', `StandardOutput=append:${logPath}`, `StandardError=append:${logPath}`, '',
    '[Install]', 'WantedBy=default.target', ''].join('\n');
}

export function renderServicePlist(plan: ServicePlan): string {
  const logPath = xml(join(plan.home, 'server.log'));
  return ['<?xml version="1.0" encoding="UTF-8"?>',
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">',
    '<plist version="1.0">', '<dict>', '<key>Label</key>', `<string>${SERVICE_LABEL}</string>`,
    '<key>ProgramArguments</key>', '<array>',
    ...plan.program.map(value => `<string>${xml(value)}</string>`), '</array>',
    '<key>EnvironmentVariables</key>', '<dict>', '<key>PATH</key>', `<string>${xml(plan.environmentPath)}</string>`,
    '<key>FRAMERIZE_SERVER_HOME</key>', `<string>${xml(plan.home)}</string>`, '</dict>',
    '<key>WorkingDirectory</key>', `<string>${xml(plan.homeDir)}</string>`,
    '<key>RunAtLoad</key>', '<true/>', '<key>KeepAlive</key>', '<true/>',
    '<key>ThrottleInterval</key>', '<integer>5</integer>', '<key>ExitTimeOut</key>', '<integer>90</integer>',
    '<key>StandardOutPath</key>', `<string>${logPath}</string>`,
    '<key>StandardErrorPath</key>', `<string>${logPath}</string>`, '</dict>', '</plist>', ''].join('\n');
}

export function selectServiceManager(plan: ServicePlan): ServiceManager | undefined {
  if ([plan.home, plan.homeDir, ...plan.program].some(value => /[\x00-\x1f\x7f]/.test(value))) {
    throw new Error('Service paths must not contain control characters');
  }
  const systemctl = (...args: string[]): ServiceStep => ({ command: 'systemctl', args: ['--user', ...args] });
  if (plan.platform === 'linux') {
    const reload = systemctl('daemon-reload');
    const configHome = plan.xdgConfigHome && isAbsolute(plan.xdgConfigHome)
      ? plan.xdgConfigHome : join(plan.homeDir, '.config');
    return {
      path: join(configHome, 'systemd', 'user', SERVICE_UNIT),
      contents: renderServiceUnit(plan),
      activate: [reload, systemctl('enable', SERVICE_UNIT), systemctl('restart', SERVICE_UNIT)],
      stop: [systemctl('stop', SERVICE_UNIT)], restart: [systemctl('restart', SERVICE_UNIT)],
      remove: [systemctl('disable', '--now', SERVICE_UNIT)], reload: [reload],
      enable: [systemctl('enable', SERVICE_UNIT)],
      active: systemctl('show', SERVICE_UNIT, '--property=ActiveState,SubState,MainPID'),
      enabled: systemctl('is-enabled', SERVICE_UNIT),
    };
  }
  if (plan.platform !== 'darwin') return undefined;
  const path = join(plan.homeDir, 'Library', 'LaunchAgents', `${SERVICE_LABEL}.plist`);
  const domain = `gui/${plan.uid}`;
  const target = `${domain}/${SERVICE_LABEL}`;
  const launchctl = (...args: string[]): ServiceStep => ({ command: 'launchctl', args });
  const stop = launchctl('bootout', '--wait', target);
  const start = launchctl('bootstrap', domain, path);
  return { path, contents: renderServicePlist(plan),
    activate: [{ ...launchctl('enable', target), optional: true }, start],
    stop: [stop], restart: [stop, start], remove: [stop], reload: [], enable: [launchctl('enable', target)],
    active: launchctl('print', target) };
}
