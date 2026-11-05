import type { ClientCommand, ClientName } from './clients.js';
import { CLIENTS } from './clients.js';

// Path classification adapted from T3 Code packages/provider-core/src/server/maintenanceResolver.ts
// and apps/server/src/provider/Drivers/{ClaudeMaintenance,CodexDriver}.ts at 5a906583 (MIT).

export function normalizeCommandPath(commandPath: string): string {
  return commandPath.replaceAll('\\', '/').toLowerCase();
}

export function npmGlobalPrefixFromCommandPath(realPath: string, packageName: string): string | null {
  const slashPath = realPath.replaceAll('\\', '/');
  const normalized = slashPath.toLowerCase();
  const index = normalized.lastIndexOf(`/lib/node_modules/${packageName.toLowerCase()}/`);
  if (index < 0 || normalized.slice(0, index).includes('/node_modules/')) return null;
  const miseTool = /\/mise\/installs\/([^/]+)\/[^/]+$/.exec(normalized.slice(0, index))?.[1];
  if (miseTool && miseTool !== 'node') return null;
  return index === 0 ? '/' : slashPath.slice(0, index);
}

export function homebrewCaskFromCommandPath(realPath: string): string | null {
  const match = /^(.*)\/caskroom\/([^/]+)\/[^/]+\//i.exec(realPath.replaceAll('\\', '/'));
  return match ? match[2]! : null;
}

export function codexStandaloneHome(commandPath: string): string | null {
  const match = /^(.*)\/packages\/standalone\//is.exec(commandPath.replaceAll('\\', '/'));
  if (!match) return null;
  const home = match[1]!;
  const isRoot = home === '' || /^[a-z]:$/i.test(home);
  return commandPath.slice(0, home.length + (isRoot ? 1 : 0));
}

function isNativeClaude(commandPath: string): boolean {
  const normalized = normalizeCommandPath(commandPath);
  return normalized.endsWith('/.local/bin/claude') || normalized.endsWith('/.local/bin/claude.exe')
    || normalized.includes('/.local/share/claude/');
}

export interface InstallOrigin {
  readonly resolvedPath: string;
  readonly realPath: string;
  readonly platform: NodeJS.Platform;
  readonly env: NodeJS.ProcessEnv;
}

/** Which updater owns the install. The latest-version lookup follows the same owner. */
export type UpdateOwner = { readonly kind: 'native' | 'bun' | 'mise' | 'winget' | 'unknown' }
  | { readonly kind: 'npm'; readonly prefix: string }
  | { readonly kind: 'homebrew'; readonly cask: string }
  | { readonly kind: 'chocolatey' };

/** An update the tool can run, or the reason it only names a manual step. */
export type UpdatePlan =
  | { readonly kind: 'command'; readonly command: ClientCommand; readonly owner: UpdateOwner }
  | { readonly kind: 'manual'; readonly reason: string; readonly owner: UpdateOwner };

const run = (owner: UpdateOwner, command: string, args: readonly string[], env?: Record<string, string>): UpdatePlan =>
  ({ kind: 'command', owner, command: { command, args, display: [command, ...args].join(' '), ...(env ? { env } : {}) } });
const manual = (owner: UpdateOwner, reason: string): UpdatePlan => ({ kind: 'manual', reason, owner });

function isMise(paths: readonly string[]): boolean {
  return paths.some(path => /\/mise\/(?:installs|shims|command-wrappers)\/|\/mise(?:\.exe)?$/
    .test(normalizeCommandPath(path)));
}

/** Uses the canonical install owner before interpreting native or Bun launcher aliases. */
export function planUpdate(client: ClientName, origin: InstallOrigin): UpdatePlan {
  const paths = [origin.resolvedPath, origin.realPath];
  const npmPackage = CLIENTS[client].npmPackage;
  const normalized = paths.map(normalizeCommandPath);
  if (client === 'claude' && origin.platform === 'win32') {
    if (normalized.some(path => /\/winget\/(?:links|packages\/anthropic\.claudecode_[^/]+)\/claude\.exe$/.test(path))) {
      return manual({ kind: 'winget' }, 'winget upgrade Anthropic.ClaudeCode');
    }
    if (normalized.some(path => /\/chocolatey\/(?:bin|lib\/claude-code\/tools)\/claude\.exe$/.test(path))) {
      return manual({ kind: 'chocolatey' }, 'choco upgrade claude-code');
    }
  }
  const cask = homebrewCaskFromCommandPath(origin.realPath);
  if (cask) return manual({ kind: 'homebrew', cask }, `brew upgrade --cask ${cask}`);
  const npmPrefix = npmGlobalPrefixFromCommandPath(origin.realPath, npmPackage);
  if (client === 'claude' && paths.some(isNativeClaude) && !npmPrefix && !isMise(paths)) {
    return run({ kind: 'native' }, origin.resolvedPath, ['update']);
  }
  const codexHome = client === 'codex'
    ? codexStandaloneHome(origin.realPath) ?? codexStandaloneHome(origin.resolvedPath) : null;
  if (codexHome && !npmPrefix && !isMise(paths)) return run({ kind: 'native' }, origin.resolvedPath, ['update'], { CODEX_HOME: codexHome });
  if (normalized.some(path => path.includes('/.bun/bin/'))) return run({ kind: 'bun' }, 'bun', ['i', '-g', `${npmPackage}@latest`]);
  if (npmPrefix) {
    return run({ kind: 'npm', prefix: npmPrefix }, 'npm', ['install', '-g', '--prefix', npmPrefix, `--allow-scripts=${npmPackage}`, `${npmPackage}@latest`]);
  }
  if (isMise(paths)) return manual({ kind: 'mise' }, 'Update the version pinned in your mise configuration.');
  return manual({ kind: 'unknown' }, `Follow ${CLIENTS[client].docs} for the method that installed ${origin.resolvedPath}.`);
}
