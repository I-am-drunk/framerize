import { CLIENTS, type ClientName } from './clients.js';
import { codexBaseUrl, claudeBaseUrl, launchCommand } from './configure.js';
import { confirm, defaultRealPath, EXIT, lookupDeps, type ClientSetupDeps } from './deps.js';
import { checkVersion } from './latest.js';
import { conflictText, inspectClaudeSettings, withSettingsOverride } from './conflicts.js';
import { readText, versionLine } from './versions.js';
import { installCommand } from './install.js';
import { planUpdate } from './maintenance.js';
import { parseClientOptions, type ClientOptions } from './options.js';
import { probeExecutable, resolveCommandPath } from './platform.js';
import { claudeSettingsPath, SettingsError, undoClaudeSettings, writeClaudeSettings } from './settings.js';

export const CLIENT_HELP = `Usage: frm client <claude|codex> [options] [-- client arguments]

  --config FILE Use this Remote profile instead of the default
  --check      Report the OS, client path, version and Remote address without changes
  --install    Consent to install a missing client with its official installer
  --update     Update an installed client after a separate confirmation
  --persist    Claude only: write the Remote address to Claude settings
  --undo       Claude only: remove the values --persist wrote
  --no-input   Never prompt. Missing consent exits with code 3

Without --check, --persist or --undo, the client starts against the local Remote.
Arguments after -- pass to the client unchanged, including --model.
FRM_CLIENT_INSTALL=1 and FRM_NO_INPUT=1 match --install and --no-input.
Updates need --update on the command itself.
`;

export function locate(client: ClientName, deps: ClientSetupDeps): string | null {
  return resolveCommandPath(CLIENTS[client].command, deps.platform, deps.env, deps.probe ?? probeExecutable);
}

/** Without a terminal the explicit flag is the consent. A terminal confirms update by typing the word. */
async function consented(options: ClientOptions, deps: ClientSetupDeps, flag: boolean,
  question: string, word: 'install' | 'update'): Promise<boolean> {
  if (options.noInput || !deps.prompt) return flag;
  if (word === 'install' && flag) return true;
  return confirm(deps.prompt, question, word);
}

async function ensureInstalled(options: ClientOptions, deps: ClientSetupDeps): Promise<string | number> {
  const found = locate(options.client, deps);
  if (found) return found;
  const { label, docs } = CLIENTS[options.client];
  const install = installCommand(options.client, deps.platform);
  if (!install) {
    deps.io.stderr(`${label} was not found on PATH. Install it for ${deps.platform} from ${docs}\n`);
    return EXIT.failed;
  }
  deps.io.stdout(`${label} was not found on PATH.\nOfficial installer: ${install.display}\n`);
  if (!await consented(options, deps, options.install, `Install ${label}?`, 'install')) {
    deps.io.stderr(`${label} was not installed. Rerun with --install or FRM_CLIENT_INSTALL=1 to consent.\n`);
    return EXIT.needsConsent;
  }
  const code = await deps.runner.run(install, deps.env);
  if (code !== 0) { deps.io.stderr(`${label} installer exited with code ${code}.\n`); return EXIT.failed; }
  const installed = locate(options.client, deps);
  if (installed) return installed;
  deps.io.stderr(`${label} installed but is not on PATH yet. Open a new terminal and rerun this command.\n`);
  return EXIT.failed;
}

async function updateInstalled(options: ClientOptions, deps: ClientSetupDeps, executable: string): Promise<number> {
  const { label } = CLIENTS[options.client];
  const plan = planUpdate(options.client, { resolvedPath: executable,
    realPath: (deps.realPath ?? defaultRealPath)(executable), platform: deps.platform, env: deps.env });
  if (plan.kind === 'manual') {
    deps.io.stdout(`${label} at ${executable} is updated outside Framerize: ${plan.reason}\n`);
    return EXIT.ok;
  }
  const version = await checkVersion(options.client, executable, plan, lookupDeps(deps));
  deps.io.stdout(`${label} is installed at ${executable}.\n${versionLine(version)}\n`);
  if (version.status === 'current') return EXIT.ok;
  deps.io.stdout(`Update command: ${plan.command.display}\n`);
  if (!await consented(options, deps, options.update, `Update ${label}?`, 'update')) {
    deps.io.stderr(`${label} was not updated. Run this command again and type update to confirm.\n`);
    return EXIT.needsConsent;
  }
  const code = await deps.runner.run(plan.command, deps.env);
  if (code !== 0) deps.io.stderr(`${label} update exited with code ${code}.\n`);
  return code === 0 ? EXIT.ok : EXIT.failed;
}

async function check(options: ClientOptions, deps: ClientSetupDeps): Promise<number> {
  const { label, command } = CLIENTS[options.client];
  const executable = locate(options.client, deps);
  const base = options.client === 'claude' ? claudeBaseUrl(deps.target.origin) : codexBaseUrl(deps.target.origin);
  deps.io.stdout(`OS: ${deps.platform}\n${label}: ${executable ?? 'not found on PATH'}\n`);
  if (executable) {
    const version = await deps.runner.capture({ command: executable, args: ['--version'],
      display: `${command} --version` }, deps.env);
    deps.io.stdout(`Version: ${version?.trim().split('\n')[0] ?? 'unavailable'}\n`);
  }
  deps.io.stdout(`Remote base URL: ${base}\nLocal token: ${deps.target.localToken ? 'set' : 'not set'}\n`);
  return executable ? EXIT.ok : EXIT.failed;
}

/** Remote launches use their own auth and endpoint, including when the shell selects a cloud provider. */
function launchEnv(client: ClientName, env: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  if (client !== 'claude') return env;
  const { ANTHROPIC_API_KEY: _dropped, CLAUDE_CODE_USE_BEDROCK: _bedrock,
    CLAUDE_CODE_USE_VERTEX: _vertex, CLAUDE_CODE_USE_FOUNDRY: _foundry,
    CLAUDE_CODE_USE_ANTHROPIC_AWS: _aws, ...rest } = env;
  return rest;
}

/** Standalone `frm client` handler. The parent resolves the Remote target and wires real deps. */
export async function runClient(argv: readonly string[], deps: ClientSetupDeps): Promise<number> {
  if (argv.length === 0 || ['help', '--help', '-h'].includes(argv[0]!)) { deps.io.stdout(CLIENT_HELP); return EXIT.ok; }
  let options: ClientOptions | null;
  try { options = parseClientOptions(argv, deps.env); } catch { options = null; }
  if (!options) { deps.io.stderr('Invalid client options. Run frm client --help.\n'); return EXIT.usage; }
  if (options.check) return check(options, deps);
  try {
    if (options.undo) {
      const result = await undoClaudeSettings(deps.stateDir);
      deps.io.stdout(result === 'restored' ? 'Removed the Remote values from Claude settings.\n'
        : result === 'changed_since' ? 'Claude settings changed since Framerize wrote them; left them unchanged.\n'
          : 'Framerize has not written Claude settings.\n');
      return EXIT.ok;
    }
    if (options.persist) {
      const path = claudeSettingsPath(deps.env, deps.home);
      await writeClaudeSettings(path, deps.stateDir, deps.target);
      const undo = options.config === undefined ? 'frm client claude --undo'
        : 'use --undo instead of --persist, keeping the same --config';
      deps.io.stdout(`Claude settings now use ${claudeBaseUrl(deps.target.origin)}: ${path}\nUndo: ${undo}\n`);
      return EXIT.ok;
    }
  } catch (error) {
    if (!(error instanceof SettingsError)) throw error;
    deps.io.stderr(`${error.message}\n`);
    return EXIT.failed;
  }
  if (options.update) {
    const executable = locate(options.client, deps);
    if (executable) return updateInstalled(options, deps, executable);
    deps.io.stderr(`${CLIENTS[options.client].label} is not installed. Rerun with --install.\n`);
    return EXIT.failed;
  }
  const executable = await ensureInstalled(options, deps);
  if (typeof executable === 'number') return executable;
  let launch = launchCommand(options.client, executable, deps.target, options.passthrough);
  if (options.client === 'claude') {
    const settings = await inspectClaudeSettings(deps.read ?? readText, { platform: deps.platform,
      env: deps.env, userPath: claudeSettingsPath(deps.env, deps.home), cwd: deps.cwd ?? process.cwd(),
      expected: launch.env ?? {} });
    if (settings.blocking.length > 0) {
      deps.io.stderr(conflictText(settings.blocking));
      return EXIT.failed;
    }
    // Command-line settings outrank user and project files, so a foreign base URL cannot win.
    if (settings.overrideBaseUrl) launch = withSettingsOverride(launch, claudeBaseUrl(deps.target.origin));
  }
  return deps.runner.run(launch, launchEnv(options.client, deps.env));
}
