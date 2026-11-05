// Latest-version lookup order adapted from T3 Code at 5a906583 (MIT):
// apps/server/src/provider/Drivers/ClaudeMaintenance.ts resolveClaudeMaintenance (native channel URL),
// packages/provider-core/src/server/maintenanceResolver.ts withConfiguredNpmVersionLookup,
// resolveHomebrewProviderMaintenance and the Chocolatey lookup in resolvePackageManagedProviderMaintenance.
import { CLIENTS, type ClientName } from './clients.js';
import type { ReadText } from './conflicts.js';
import type { UpdatePlan } from './maintenance.js';
import { resolveCommandPath, type ExecutableProbe } from './platform.js';
import type { ProcessRunner } from './process.js';
import { claudeSettingsPath } from './settings.js';
import { compareSemverVersions } from './semver.js';
import { claudeReleaseUrl, claudeUpdateChannel, isNativeVersion, parseHomebrewCaskVersion,
  parseInstalledVersion, parseNpmViewVersion, versionStatus, type FetchText, type VersionCheck } from './versions.js';

export interface LookupDeps {
  readonly platform: NodeJS.Platform;
  readonly env: NodeJS.ProcessEnv;
  readonly home: string;
  readonly runner: ProcessRunner;
  readonly fetch: FetchText;
  readonly read: ReadText;
  readonly probe?: ExecutableProbe;
}

const capture = async (deps: LookupDeps, tool: string, args: readonly string[]): Promise<string | null> => {
  const command = resolveCommandPath(tool, deps.platform, deps.env, deps.probe);
  return command ? deps.runner.capture({ command, args, display: [tool, ...args].join(' ') }, deps.env) : null;
};

/** `choco search --exact --limit-output` prints `<id>|<version>` for a match. */
export function parseChocolateyVersion(stdout: string | null, id: string): string | null {
  for (const line of (stdout ?? '').split(/\r?\n/)) {
    const [name, version] = line.trim().split('|', 2);
    if (name?.toLowerCase() === id && version?.trim()) return version.trim();
  }
  return null;
}

const RELEASE_TIMEOUT_MS = 4000;

/**
 * Asks the owner of an install for its newest release. Every failure returns null (unknown).
 * Like T3, npm lookups go through the user's npm so its registry, auth and proxy apply.
 */
export async function lookupLatestVersion(client: ClientName, plan: UpdatePlan, deps: LookupDeps): Promise<string | null> {
  try {
    const npmPackage = CLIENTS[client].npmPackage;
    const owner = plan.owner;
    if (owner.kind === 'native' && client === 'claude') {
      const channel = await claudeUpdateChannel(deps.read, deps.platform, deps.env, claudeSettingsPath(deps.env, deps.home));
      const text = (await deps.fetch(claudeReleaseUrl(channel), RELEASE_TIMEOUT_MS))?.trim() ?? null;
      return isNativeVersion(text) ? text : null;
    }
    if (owner.kind === 'homebrew') {
      return parseHomebrewCaskVersion(await capture(deps, 'brew', ['info', '--json=v2', '--cask', owner.cask]));
    }
    if (owner.kind === 'chocolatey') {
      return parseChocolateyVersion(await capture(deps, 'choco', ['search', 'claude-code', '--exact', '--limit-output']), 'claude-code');
    }
    if (owner.kind === 'mise' || owner.kind === 'winget') return null;
    const prefix = owner.kind === 'npm' ? ['--prefix', owner.prefix] : [];
    return parseNpmViewVersion(await capture(deps, 'npm', ['view', '--global', ...prefix, '--json', `${npmPackage}@latest`, 'version']));
  } catch { return null; }
}

/** Installed version, latest version and status for one owned install. Never throws. */
export async function checkVersion(client: ClientName, executable: string, plan: UpdatePlan,
  deps: LookupDeps): Promise<VersionCheck> {
  let installed: string | null = null;
  try {
    installed = parseInstalledVersion(await deps.runner.capture({ command: executable, args: ['--version'],
      display: `${CLIENTS[client].command} --version` }, deps.env));
  } catch {}
  const latest = installed ? await lookupLatestVersion(client, plan, deps) : null;
  return versionStatus(installed, latest, compareSemverVersions);
}
