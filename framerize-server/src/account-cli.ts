// Account and guided configuration commands. Output shape adapts T3 Code
// apps/server/src/cliAuthFormat.ts at fbe5df2d (MIT, T3 Tools Inc.): JSON omits
// credentials and the human list prints one indented block per record.
import { resolve } from 'node:path';
import { AccountCommandError, CliUsageError, ConfigurationError } from './errors.js';
import { parseAccountList } from './accounts.js';
import { configuredSettings } from './settings.js';
import { ENVIRONMENT } from './config.js';
import { rollbackError } from './service-control.js';
import { findAccount, readRawAccounts, resolveStore, restoreAccounts, saveSettings, selectedAccount,
  writeAccounts, type RawAccount, type RawAccounts, type StorePaths } from './account-store.js';

type Environment = Readonly<Record<string, string | undefined>>;
export interface AccountCliDependencies {
  readonly stdout: { write(text: string): unknown };
  readonly env: Environment;
  readonly ask?: (question: string) => Promise<string>;
  readonly askHidden?: (question: string) => Promise<string>;
  readonly readStdin?: () => Promise<string>;
  readonly signal?: AbortSignal;
  // Deferred browser sign-in hook. The parent wires browser-login.ts here lazily.
  readonly login?: (signal?: AbortSignal) => Promise<string>;
}

const VALUE_FLAGS = ['config', 'accounts-file', 'project-id', 'label', 'model', 'session-env',
  'session-file', 'backend', 'upstream', 'authorization-env', 'authorization-file', 'account-id'];
const BOOLEAN_FLAGS = ['json', 'session-stdin', 'select', 'show'];

function parse(args: readonly string[]) {
  const positional: string[] = [];
  const values: Record<string, string> = {};
  const flags = new Set<string>();
  for (let i = 0; i < args.length; i++) {
    const arg = args[i]!;
    if (!arg.startsWith('--')) { positional.push(arg); continue; }
    const name = arg.slice(2);
    if (BOOLEAN_FLAGS.includes(name)) { flags.add(name); continue; }
    const value = args[++i];
    if (!VALUE_FLAGS.includes(name) || !value || value.startsWith('--')) throw new CliUsageError('Invalid option');
    values[name.replace(/-([a-z])/g, (_match, letter: string) => letter.toUpperCase())] = value;
  }
  return { positional, values, flags };
}

// References are names and paths, never values. An inline session prints only its kind.
function sessionView(session: unknown) {
  if (session === undefined) return 'missing';
  if (typeof session === 'string') return 'saved';
  const reference = session as { env?: string; file?: string };
  return reference.env ? { env: reference.env } : { file: String(reference.file) };
}

function view(account: RawAccount, selected: string | undefined) {
  const { id, label, projectId, model, models } = account;
  return { id, enabled: account.enabled !== false, selected: id === selected,
    ...(label !== undefined ? { label } : {}), ...(projectId !== undefined ? { projectId } : {}),
    ...(model !== undefined ? { model } : {}), ...(models !== undefined ? { models } : {}),
    session: sessionView(account.session) };
}

type View = ReturnType<typeof view>;
function describe(account: View): string {
  const session = typeof account.session === 'string' ? account.session
    : 'env' in account.session ? `env ${account.session.env}` : `file ${account.session.file}`;
  return [`${account.id}${account.label ? ` (${account.label})` : ''}${account.selected ? ' selected' : ''}`,
    `  enabled: ${account.enabled ? 'yes' : 'no'}`,
    `  project: ${account.projectId ?? 'missing'}`,
    `  session: ${session}`,
    ...(account.model ? [`  model: ${account.model}`] : [])].join('\n');
}

function print(out: AccountCliDependencies['stdout'], json: boolean, value: unknown, text: string) {
  out.write(json ? `${JSON.stringify(value, null, 2)}\n` : `${text}\n`);
}

export async function runAccountsCommand(argv: readonly string[], deps: AccountCliDependencies): Promise<number> {
  const action = argv[0]?.startsWith('--') ? undefined : argv[0];
  const { positional, values, flags } = parse(action ? argv.slice(1) : argv);
  if (positional.length > 1) throw new CliUsageError('Invalid option');
  const json = flags.has('json');
  const store = await resolveStore(values, deps.env);
  const raw = await readRawAccounts(store.accounts, deps.signal);
  const id = positional[0];
  if (!action && deps.ask && !json) return menu(store, raw, deps);
  if (!action || action === 'list') return list(store, raw.accounts, json, deps.stdout);
  if (action === 'show') {
    const account = view(findAccount(raw.accounts, id), selectedAccount(store));
    print(deps.stdout, json, account, describe(account));
    return 0;
  }
  if (action === 'add' || action === 'login') return add(store, raw, id, values, flags, deps, action === 'login');
  if (action === 'select' || action === 'use') return select(store, raw.accounts, id, json, deps);
  return change(action, store, raw, id, json, deps);
}

function list(store: StorePaths, accounts: readonly RawAccount[], json: boolean,
  out: AccountCliDependencies['stdout']): number {
  const selected = selectedAccount(store);
  const views = accounts.map(account => view(account, selected));
  const text = views.length ? views.map(describe).join('\n\n')
    : `No accounts in ${store.accounts}. Add one with: accounts add ID --project-id PROJECT --session-stdin`;
  print(out, json, { accountsFile: store.accounts, selected: selected ?? null, accounts: views }, text);
  return 0;
}

function usable(account: RawAccount): boolean {
  return account.enabled !== false && typeof account.projectId === 'string' && account.session !== undefined;
}

// A session arrives by env or file reference, stdin or a hidden prompt, never argv.
async function sessionInput(values: Record<string, string>, flags: Set<string>,
  deps: AccountCliDependencies): Promise<unknown> {
  const sources = [values.sessionEnv, values.sessionFile, flags.has('session-stdin') || undefined]
    .filter(value => value !== undefined);
  if (sources.length > 1) throw new AccountCommandError('Choose one session source');
  if (values.sessionEnv) return { env: values.sessionEnv };
  if (values.sessionFile) return { file: resolve(values.sessionFile) };
  const read = flags.has('session-stdin') ? deps.readStdin
    : deps.askHidden && (() => deps.askHidden!('Framer session (input hidden): '));
  if (!read) throw new AccountCommandError('Pass --session-env, --session-file or --session-stdin');
  const session = (await read()).trim();
  if (!session) throw new AccountCommandError('A session is required');
  return session;
}

// Writes accounts, then settings. A failed settings write restores the prior accounts bytes.
async function commit(store: StorePaths, previous: RawAccounts, next: readonly RawAccount[],
  settings: Readonly<Record<string, unknown>> | undefined, signal?: AbortSignal): Promise<void> {
  signal?.throwIfAborted();
  const current = await readRawAccounts(store.accounts, signal);
  signal?.throwIfAborted();
  if (current.text !== previous.text) {
    throw new AccountCommandError('Accounts changed during this command. Retry the command.');
  }
  await writeAccounts(store.accounts, next, signal);
  if (!settings) return;
  try { await saveSettings(store, settings, {}, signal); }
  catch (error) {
    try { await restoreAccounts(store.accounts, previous); }
    catch (failure) { throw rollbackError(error, [failure]); }
    throw error;
  }
}

async function add(store: StorePaths, raw: RawAccounts, id: string | undefined,
  values: Record<string, string>, flags: Set<string>, deps: AccountCliDependencies, browser = false): Promise<number> {
  const accountId = id ?? (deps.ask ? (await deps.ask('Account id: ')).trim() : undefined);
  if (!accountId) throw new AccountCommandError('An account id is required');
  if (raw.accounts.some(entry => entry.id === accountId)) throw new AccountCommandError('Account id already exists');
  const projectId = values.projectId ?? (deps.ask ? (await deps.ask('Framer project id: ')).trim() : '');
  if (!projectId) throw new AccountCommandError('A Framer project id is required');
  const fields: RawAccount = { id: accountId, projectId,
    ...(values.label ? { label: values.label } : {}), ...(values.model ? { model: values.model } : {}) };
  // Reject a bad id or project before reading a session or opening a browser.
  checkNewAccount(store, [...raw.accounts, { ...fields, session: { env: 'PLACEHOLDER' } }]);
  if (browser && !deps.login) throw new AccountCommandError('Browser sign-in is unavailable here');
  const session = browser ? await deps.login!(deps.signal) : await sessionInput(values, flags, deps);
  const account: RawAccount = { ...fields, session };
  const selecting = browser || flags.has('select');
  await commit(store, raw, [...raw.accounts, account], selecting ? selection(store, accountId) : undefined,
    deps.signal);
  const shown = view(account, selecting ? accountId : selectedAccount(store));
  print(deps.stdout, flags.has('json'), shown, browser ? `Signed in ${accountId}.` : `Added ${accountId}.`);
  return 0;
}

function checkNewAccount(store: StorePaths, accounts: readonly RawAccount[]): void {
  try { parseAccountList(accounts, store.accounts); }
  catch (error) { throw error instanceof ConfigurationError ? new AccountCommandError(error.message) : error; }
}

// Selecting switches saved settings to the Framer backend, which excludes an HTTP upstream.
function selection(store: StorePaths, id: string) {
  return { backend: 'framer', accountId: id, accountsFile: store.accounts,
    upstream: undefined, authorizationEnv: undefined, authorizationFile: undefined };
}

async function select(store: StorePaths, accounts: readonly RawAccount[], id: string | undefined,
  json: boolean, deps: AccountCliDependencies): Promise<number> {
  const account = findAccount(accounts, id);
  if (!usable(account)) throw new AccountCommandError('Select an enabled account with a project and session');
  await saveSettings(store, selection(store, String(account.id)), {}, deps.signal);
  print(deps.stdout, json, { selected: account.id, settings: store.settings },
    `Selected ${account.id}. Saved in ${store.settings}.`);
  return 0;
}

async function change(action: string, store: StorePaths, raw: RawAccounts, id: string | undefined,
  json: boolean, deps: AccountCliDependencies): Promise<number> {
  if (!['enable', 'disable', 'remove'].includes(action)) throw new CliUsageError('Unknown command');
  const account = findAccount(raw.accounts, id);
  const next = action === 'remove' ? raw.accounts.filter(entry => entry !== account)
    : raw.accounts.map(entry => entry === account ? { ...entry, enabled: action === 'enable' } : entry);
  // Clearing the selection keeps the existing single-enabled-account rule for later runs.
  const clear = action !== 'enable' && selectedAccount(store) === account.id;
  await commit(store, raw, next, clear ? { accountId: undefined } : undefined, deps.signal);
  const verb = action === 'remove' ? 'Removed' : action === 'enable' ? 'Enabled' : 'Disabled';
  print(deps.stdout, json, { id: account.id, action }, `${verb} ${account.id}.`);
  return 0;
}

// A numbered menu over the same commands. Blank input or quit leaves files untouched.
async function menu(store: StorePaths, raw: RawAccounts, deps: AccountCliDependencies): Promise<number> {
  const chosen = selectedAccount(store);
  deps.stdout.write(raw.accounts.length ? raw.accounts.map((account, index) =>
    `${index + 1}. ${describe(view(account, chosen))}`).join('\n') + '\n' : `No accounts in ${store.accounts}.\n`);
  const ask = deps.ask!;
  const action = (await ask('\nAction [login/add/select/enable/disable/remove/quit] (quit): ')).trim().toLowerCase();
  if (!action || action === 'quit' || action === 'q') { deps.stdout.write('Nothing changed.\n'); return 0; }
  if (!['login', 'add', 'select', 'enable', 'disable', 'remove'].includes(action)) {
    throw new AccountCommandError('Choose login, add, select, enable, disable, remove or quit');
  }
  if (action === 'add' || action === 'login') {
    return add(store, raw, undefined, {}, new Set(), deps, action === 'login');
  }
  const answer = (await ask('Account number or id: ')).trim();
  const index = /^\d+$/.test(answer) ? Number(answer) - 1 : -1;
  const id = String(raw.accounts[index]?.id ?? answer);
  if (action === 'remove' && !/^y(es)?$/i.test((await ask(`Remove ${id}? [y/N]: `)).trim())) {
    deps.stdout.write('Nothing changed.\n');
    return 0;
  }
  return action === 'select' ? select(store, raw.accounts, id, false, deps) : change(action, store, raw, id, false, deps);
}

// Effective settings after flags, environment and saved defaults. Credentials appear only as names.
async function effective(store: StorePaths, values: Record<string, string>, env: AccountCliDependencies['env']) {
  const flags = Object.fromEntries(Object.entries(values).filter(([name]) => Object.hasOwn(ENVIRONMENT, name)));
  const loaded = await configuredSettings(store.saved ? store.settings : undefined, flags, env);
  // A literal token from saved config or the environment never leaves this function.
  const { serverToken: _token, ...config } = { ...loaded.config, serverToken: '' };
  const source = (name: string) => Object.hasOwn(flags, name) ? 'flag'
    : env[ENVIRONMENT[name as keyof typeof ENVIRONMENT]] !== undefined ? 'environment'
      : store.saved && Object.hasOwn(store.saved.config, name) ? 'saved' : 'default';
  return { settingsFile: store.settings, saved: Boolean(store.saved),
    config, sources: Object.fromEntries(Object.keys(config).map(name => [name, source(name)])),
    serverToken: loaded.config.serverToken || loaded.config.serverTokenFile ? 'set' : 'none' };
}

// Guided choice between Framer and an HTTP upstream. Flags answer questions; prompts fill the rest.
export async function runConfigureCommand(argv: readonly string[], deps: AccountCliDependencies): Promise<number> {
  const { positional, values, flags } = parse(argv);
  if (positional.length) throw new CliUsageError('Invalid option');
  const store = await resolveStore(values, deps.env);
  const json = flags.has('json');
  if (flags.has('show')) {
    const report = await effective(store, values, deps.env);
    print(deps.stdout, json, report, [`settings: ${report.settingsFile}${report.saved ? '' : ' (not saved)'}`,
      ...Object.entries(report.config).map(([name, value]) => `${name}: ${String(value)} (${report.sources[name]})`),
      `serverToken: ${report.serverToken}`].join('\n'));
    return 0;
  }
  const asked = (question: string) => deps.ask ? deps.ask(question).then(answer => answer.trim()) : undefined;
  const current = store.saved?.config.backend === 'framer' ? 'framer' : 'http';
  const backend = (values.backend ?? await asked(`Backend [framer/http] (${current}): `)) || current;
  if (backend === 'framer') return configureFramer(store, values, json, deps, asked);
  if (backend !== 'http') throw new AccountCommandError('Choose framer or http');
  return configureHttp(store, values, json, deps, asked);
}

type Ask = (question: string) => Promise<string> | undefined;
async function configureFramer(store: StorePaths, values: Record<string, string>, json: boolean,
  deps: AccountCliDependencies, asked: Ask): Promise<number> {
  const raw = await readRawAccounts(store.accounts, deps.signal);
  const choices = raw.accounts.filter(usable);
  if (!choices.length && !values.accountId) {
    if (!deps.ask) throw new AccountCommandError('Add an account first with accounts add ID --select');
    return add(store, raw, undefined, {}, new Set(['select', ...(json ? ['json'] : [])]), deps);
  }
  const list = choices.map((account, index) => `${index + 1}. ${account.id}`).join('\n');
  const fallback = selectedAccount(store) ?? (choices.length === 1 ? String(choices[0]!.id) : '');
  const answer = values.accountId ?? await asked(`${list}\nAccount number or id${fallback ? ` (${fallback})` : ''}: `);
  const index = values.accountId === undefined && answer && /^\d+$/.test(answer) ? Number(answer) - 1 : -1;
  const id = String(choices[index]?.id ?? (answer || fallback));
  if (!id) throw new AccountCommandError('Choose an account with --account-id');
  return select(store, raw.accounts, id, json, deps);
}

// Saves the upstream URL and a credential reference. A named variable's current value is
// saved for the service, as up does; a file reference stays a path.
async function configureHttp(store: StorePaths, values: Record<string, string>, json: boolean,
  deps: AccountCliDependencies, asked: Ask): Promise<number> {
  const saved = store.saved?.config ?? {};
  const prior = values.upstream ?? deps.env.FRAMERIZE_SERVER_UPSTREAM ?? String(saved.upstream ?? '');
  const upstream = values.upstream ?? ((await asked(`Upstream inference API URL${prior ? ` (${prior})` : ''}: `)) || prior);
  if (!upstream) throw new AccountCommandError('Pass --upstream or answer the URL prompt');
  let authorizationEnv = values.authorizationEnv;
  let authorizationFile = values.authorizationFile && resolve(values.authorizationFile);
  if (!authorizationEnv && !authorizationFile && deps.ask) {
    const answer = await asked('Credential env variable name, file path, or blank for none: ');
    if (answer && /^[A-Za-z_][A-Za-z0-9_]*$/.test(answer)) authorizationEnv = answer;
    else if (answer) authorizationFile = resolve(answer);
  }
  return saveHttp(store, upstream, authorizationEnv, authorizationFile, json, deps);
}

async function saveHttp(store: StorePaths, upstream: string, authorizationEnv: string | undefined,
  authorizationFile: string | undefined, json: boolean, deps: AccountCliDependencies): Promise<number> {
  const value = authorizationEnv ? deps.env[authorizationEnv] : undefined;
  await saveSettings(store, { backend: 'http', upstream, accountId: undefined,
    authorizationEnv: authorizationEnv ?? '', authorizationFile: authorizationFile ?? '' },
  value !== undefined ? { [authorizationEnv!]: value } : {}, deps.signal);
  const credential = authorizationFile ? { file: authorizationFile }
    : authorizationEnv ? { env: authorizationEnv, saved: value !== undefined } : 'none';
  print(deps.stdout, json, { backend: 'http', upstream, credential, settings: store.settings },
    `Saved HTTP upstream ${upstream} in ${store.settings}.`
    + (authorizationEnv && value === undefined ? `\nSet ${authorizationEnv} before serve or up.` : ''));
  return 0;
}
