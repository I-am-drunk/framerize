import { parseArgs } from '../cliArgs.js';
import { isClientName, type ClientName } from './clients.js';

export const CLIENT_SPEC = { install: 'boolean', update: 'boolean', persist: 'boolean', undo: 'boolean',
  check: 'boolean', 'no-input': 'boolean' } as const;

export interface ClientOptions {
  readonly client: ClientName;
  /** Consent to install a missing client. Never consent to update. */
  readonly install: boolean;
  /** Request to update an installed client. Separate from install consent. */
  readonly update: boolean;
  readonly persist: boolean;
  readonly undo: boolean;
  readonly check: boolean;
  /** Never prompt. Missing consent ends the command instead. */
  readonly noInput: boolean;
  /** Arguments after `--`, passed to the client unchanged. */
  readonly passthrough: readonly string[];
}

const truthy = (value: string | undefined): boolean => value === '1' || value === 'true' || value === 'yes';

/** Parses `frm client <claude|codex> [options] [-- client args]`. FRM_CLIENT_INSTALL and FRM_NO_INPUT add consent. */
export function parseClientOptions(argv: readonly string[], env: NodeJS.ProcessEnv): ClientOptions | null {
  const [client, ...rest] = argv;
  if (!isClientName(client)) return null;
  const split = rest.indexOf('--');
  const own = split < 0 ? rest : rest.slice(0, split);
  const { options } = parseArgs(own, CLIENT_SPEC);
  const parsed = {
    client, install: options.install === true || truthy(env.FRM_CLIENT_INSTALL),
    // Update is never ambient: only the flag requests it, and a terminal still confirms it.
    update: options.update === true,
    persist: options.persist === true, undo: options.undo === true, check: options.check === true,
    noInput: options['no-input'] === true || truthy(env.FRM_NO_INPUT),
    passthrough: split < 0 ? [] : rest.slice(split + 1),
  };
  const modes = [parsed.persist, parsed.undo, parsed.check].filter(Boolean).length;
  if (modes > 1 || ((parsed.persist || parsed.undo) && client !== 'claude')) return null;
  // Ambient FRM_CLIENT_INSTALL is ignored outside a launch; explicit flags there are a usage error.
  const explicitConsent = options.install === true || options.update === true;
  if (modes > 0 && (parsed.passthrough.length > 0 || explicitConsent)) return null;
  if (parsed.update && parsed.passthrough.length > 0) return null;
  return parsed;
}
