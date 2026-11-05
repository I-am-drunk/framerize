// Adapted from T3 Code apps/server/src/cloud/bootService.ts at 5a906583 (MIT, T3 Tools Inc.).
export const SERVICE_UNIT_FILE = 'frm.service';
export const SERVICE_LAUNCHD_LABEL = 'dev.framerize.frm.service';
export const SERVICE_PLIST_FILE = `${SERVICE_LAUNCHD_LABEL}.plist`;

export interface ServicePlan {
  readonly program: readonly string[];
  readonly home: string;
  readonly logPath: string;
}

/** systemd expands `%` specifiers, including in unquoted append-log paths. */
function escapeSystemdSpecifiers(value: string): string {
  return value.replaceAll('%', '%%');
}

function quoteSystemdValue(value: string): string {
  const escaped = escapeSystemdSpecifiers(value);
  return /[\s"'\\]/.test(escaped)
    ? `"${escaped.replaceAll('\\', '\\\\').replaceAll('"', '\\"')}"` : escaped;
}

const unquoteSystemdValue = (raw: string) => (raw.startsWith('"') && raw.endsWith('"')
  ? raw.slice(1, -1).replaceAll('\\"', '"').replaceAll('\\\\', '\\') : raw).replaceAll('%%', '%');

const escapeXmlText = (value: string) =>
  value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
const unescapeXmlText = (value: string) =>
  value.replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&amp;', '&');

/** The FRM_HOME an installed unit or plist serves, so another home's service is left alone. */
export function serviceHomeOf(contents: string): string | undefined {
  const systemd = /^Environment=FRM_HOME=(.*)$/m.exec(contents)?.[1];
  if (systemd !== undefined) return unquoteSystemdValue(systemd.trim());
  const plist = /<key>FRM_HOME<\/key>\s*<string>([^<]*)<\/string>/.exec(contents)?.[1];
  return plist === undefined ? undefined : unescapeXmlText(plist);
}

/** Service units cannot rely on the user's shell or PATH. Adapted from `renderBootServiceUnit`. */
export function renderServiceUnit(plan: ServicePlan): string {
  return ['[Unit]', 'Description=Framerize Remote', 'StartLimitIntervalSec=300', 'StartLimitBurst=5', '',
    '[Service]', 'Type=simple', 'WorkingDirectory=%h',
    `Environment=FRM_HOME=${quoteSystemdValue(plan.home)}`,
    `ExecStart=${plan.program.map(quoteSystemdValue).join(' ')}`,
    'Restart=always', 'RestartSec=5',
    `StandardOutput=append:${escapeSystemdSpecifiers(plan.logPath)}`,
    `StandardError=append:${escapeSystemdSpecifiers(plan.logPath)}`, '',
    '[Install]', 'WantedBy=default.target', ''].join('\n');
}

/** Adapted from `renderBootServicePlist`, without the T3 launcher protocol keys. */
export function renderServicePlist(plan: ServicePlan, options: { homeDir: string; environmentPath: string }): string {
  return ['<?xml version="1.0" encoding="UTF-8"?>',
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">',
    '<plist version="1.0">', '<dict>', '  <key>Label</key>', `  <string>${SERVICE_LAUNCHD_LABEL}</string>`,
    '  <key>ProgramArguments</key>', '  <array>',
    ...plan.program.map((argument) => `    <string>${escapeXmlText(argument)}</string>`), '  </array>',
    '  <key>EnvironmentVariables</key>', '  <dict>', '    <key>PATH</key>',
    `    <string>${escapeXmlText(options.environmentPath)}</string>`, '    <key>FRM_HOME</key>',
    `    <string>${escapeXmlText(plan.home)}</string>`, '  </dict>', '  <key>WorkingDirectory</key>',
    `  <string>${escapeXmlText(options.homeDir)}</string>`, '  <key>RunAtLoad</key>', '  <true/>',
    '  <key>KeepAlive</key>', '  <true/>', '  <key>ThrottleInterval</key>', '  <integer>5</integer>',
    '  <key>StandardOutPath</key>', `  <string>${escapeXmlText(plan.logPath)}</string>`,
    '  <key>StandardErrorPath</key>', `  <string>${escapeXmlText(plan.logPath)}</string>`,
    '</dict>', '</plist>', ''].join('\n');
}
