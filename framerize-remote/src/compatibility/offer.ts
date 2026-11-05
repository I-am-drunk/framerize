import { CLIENTS, type ClientName } from './clients.js';
import { locate } from './command.js';
import { confirm, defaultRealPath, lookupDeps, type ClientSetupDeps } from './deps.js';
import { checkVersion } from './latest.js';
import { versionLine, type VersionStatus } from './versions.js';
import { installCommand } from './install.js';
import { planUpdate } from './maintenance.js';

export type OfferState = 'installed' | 'present' | 'updated' | 'declined' | 'failed' | 'unsupported';
export interface OfferResult { readonly client: ClientName; readonly state: OfferState; readonly version?: VersionStatus }

async function offerInstall(client: ClientName, deps: ClientSetupDeps): Promise<OfferState> {
  const { label, docs } = CLIENTS[client];
  const install = installCommand(client, deps.platform);
  if (!install) { deps.io.stdout(`${label}: not found. Install it for ${deps.platform} from ${docs}\n`); return 'unsupported'; }
  deps.io.stdout(`${label}: not found on PATH.\nOfficial installer: ${install.display}\n`);
  if (!await confirm(deps.prompt, `Install ${label}?`, 'install', deps.signal)
    || deps.signal?.aborted) return 'declined';
  const code = await deps.runner.run(install, deps.env);
  if (code !== 0) { deps.io.stderr(`${label} installer exited with code ${code}.\n`); return 'failed'; }
  if (locate(client, deps)) return 'installed';
  deps.io.stderr(`${label} installed but is not on PATH yet. Open a new terminal.\n`);
  return 'failed';
}

/** Offers an update only when the owner reports a newer release, after a separate typed `update`. */
async function offerUpdate(client: ClientName, deps: ClientSetupDeps, executable: string): Promise<OfferResult> {
  const { label } = CLIENTS[client];
  const plan = planUpdate(client, { resolvedPath: executable,
    realPath: (deps.realPath ?? defaultRealPath)(executable), platform: deps.platform, env: deps.env });
  const version = await checkVersion(client, executable, plan, lookupDeps(deps));
  const present = { client, state: 'present', version: version.status } as const;
  if (deps.signal?.aborted) return present;
  deps.io.stdout(`${label}: ${executable}\n${versionLine(version)}\n`);
  if (version.status !== 'behind') return present;
  if (plan.kind === 'manual') { deps.io.stdout(`To update ${label}: ${plan.reason}\n`); return present; }
  if (!await confirm(deps.prompt, `Update ${label} with ${plan.command.display}?`, 'update', deps.signal)
    || deps.signal?.aborted) return present;
  const code = await deps.runner.run(plan.command, deps.env);
  if (code !== 0) deps.io.stderr(`${label} update exited with code ${code}.\n`);
  return { client, state: code === 0 ? 'updated' : 'failed', version: version.status };
}

/**
 * Setup-only offer for both clients, for use after an interactive profile setup succeeds.
 * It never starts a client. Without a terminal prompt it reports state and changes nothing.
 */
export async function offerClients(deps: ClientSetupDeps,
  clients: readonly ClientName[] = ['claude', 'codex']): Promise<OfferResult[]> {
  const results: OfferResult[] = [];
  for (const client of clients) {
    if (deps.signal?.aborted) break;
    const executable = locate(client, deps);
    results.push(executable ? await offerUpdate(client, deps, executable)
      : { client, state: await offerInstall(client, deps) });
  }
  if (!deps.prompt && !deps.signal?.aborted) {
    deps.io.stdout('No terminal: nothing was installed or updated. Use frm client <name> --install.\n');
  }
  return results;
}
