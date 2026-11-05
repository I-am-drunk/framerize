/**
 * `frm client` compatibility setup for Claude Code and Codex against a local Remote.
 * The parent CLI owns dispatch, Remote target resolution and real dependency wiring.
 */
export { runClient, CLIENT_HELP } from './command.js';
export { EXIT, confirm, defaultRealPath, terminalPrompt } from './deps.js';
export type { ClientIo, ClientSetupDeps, Prompt } from './deps.js';
export type { RemoteTarget } from './configure.js';
export { claudeBaseUrl, codexBaseUrl, codexOverrides, launchCommand, LOCAL_TOKEN_ENV } from './configure.js';
export { nodeRunner } from './process.js';
export type { ProcessRunner } from './process.js';
export { CLIENTS } from './clients.js';
export type { ClientCommand, ClientName } from './clients.js';
export { planUpdate } from './maintenance.js';
export type { UpdatePlan } from './maintenance.js';
export { npmGlobalPrefixFromCommandPath, codexStandaloneHome } from './maintenance.js';
export { resolveCommandPath, windowsPathExtensions } from './platform.js';
export { escapeWindowsShellArg, spawnPlan } from './process.js';
export { parseClientOptions } from './options.js';
export { claudeSettingsPath, writeClaudeSettings, undoClaudeSettings, SettingsError } from './settings.js';
export { offerClients } from './offer.js';
export type { OfferResult, OfferState } from './offer.js';
export { locate } from './command.js';
export { fetchText, readText, versionStatus, versionLine, parseInstalledVersion } from './versions.js';
export type { FetchText, VersionCheck, VersionStatus } from './versions.js';
export type { ReadText, SettingsConflict, LaunchSettings } from './conflicts.js';
export { inspectClaudeSettings, claudeManagedSettingsPath } from './conflicts.js';
export { checkVersion, lookupLatestVersion } from './latest.js';
export { compareSemverVersions } from './semver.js';
export type { UpdateOwner } from './maintenance.js';
