// Inert exact policy-expression fixtures. UNRUN by the source-only author.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const source = readFileSync(new URL('../../scripts/contextual_authorization.py', import.meta.url), 'utf8');
const marker = 'COMPARISON_POLICY = """';
const start = source.indexOf(marker);
assert.ok(start >= 0);
assert.equal(source.indexOf(marker, start + marker.length), -1);
const end = source.indexOf('"""', start + marker.length);
const literal = source.slice(start + marker.length, end);
assert.equal(literal.includes('\\'), false);
const ENGINE = 'https://app.framerstatic.com/chunk-WC34TENX.mjs';
const POLICY = 'https://app.framerstatic.com/chunk-3W5IVLGF.mjs';
const PROJECT = 'fixture-project', PAGE = 'https://framer.com/projects/' + PROJECT;
const PRIVATE = 'private-policy-fixture';
const keys = ['policy_known', 'ai_disabled', 'policy_project_match',
  'policy_space_match', 'policy_page_match', 'policy_device_match', 'policy_engine_match'];
const accepted = {stage: 'policy_ready', ...Object.fromEntries(keys.map(key => [key, true])),
  ai_disabled: false};
assert.equal(literal.split('import(').length, 3);
// Only the two import operations are replaced; exact URLs and policy logic stay intact.
const expression = literal.replaceAll('import(', 'importPort(');

function fixture() {
  const space = {id: 'private-space', scope: 'organization', configuration: {disableAi: false}};
  const store = {projectId: PROJECT, project: {id: PROJECT, space: {id: space.id}}};
  const user = {spaces: [space]};
  const original = {stores: {projectStore: store, sessionStore: {user}}};
  const state = {engine: original, page: PAGE, device: PRIVATE, space, store, user,
    imports: [], calls: 0, forbidden: 0, policyResult: false, progress: []};
  const forbid = () => { state.forbidden++; throw new Error(PRIVATE); };
  const engineModule = {a: () => state.engine, b: forbid, c: forbid, d: forbid};
  const policyModule = {c: engine => {
    assert.equal(engine, original);
    state.calls++;
    state.duringPolicy?.(state);
    return state.policyResult;
  }, Pe: forbid};
  const importPort = async url => {
    assert.ok(url === ENGINE || url === POLICY);
    state.imports.push(url);
    await state.duringImport?.(url, state);
    return url === ENGINE ? engineModule : policyModule;
  };
  const location = {get href() { return state.page; }, set href(_) { forbid(); }};
  const localStorage = {getItem(key) { assert.equal(key, '__cuid'); return state.device; },
    setItem: forbid, removeItem: forbid, clear: forbid};
  const check = new Function('location', 'localStorage', 'importPort', 'fetch',
    'XMLHttpRequest', 'console', 'return (' + expression + ');')(
      location, localStorage, importPort, forbid, forbid, {debug: value => {
        if (state.progressError) throw new Error(PRIVATE);
        state.progress.push(value);
      }});
  return {state, engineModule, policyModule, run: () => check({project: PROJECT,
    page_url: PAGE, progress_key: state.progressEnabled ? 'fixture:' : undefined})};
}

test('page drift or missing device refuses before either import', async () => {
  for (const change of [s => { s.page += '/other'; }, s => { s.device = ''; }]) {
    const value = fixture(); change(value.state);
    await refused(value, 'policy_pre_context');
    assert.deepEqual(value.state.imports, []);
  }
});

test('missing engine never becomes an allowed policy', async () => {
  const value = fixture(); value.state.engine = null;
  await refused(value, 'policy_engine');
  assert.deepEqual(value.state.imports, [ENGINE]);
});

test('missing exact exports refuse without a setter or throwing preflight call', async () => {
  for (const [module, name] of [['engineModule', 'a'], ['policyModule', 'c']]) {
    const value = fixture(); value[module][name] = undefined;
    await refused(value, 'policy_export');
  }
});

test('missing or wrong current project refuses before policy evaluation', async () => {
  for (const change of [s => { s.store.projectId = 'other'; },
    s => { s.store.project = null; }, s => { s.store.project.id = 'stale'; }]) {
    const value = fixture(); change(value.state);
    await refused(value, 'policy_project');
  }
});

test('space membership and configuration must be present and unambiguous', async () => {
  for (const change of [s => { s.store.project.space = null; },
    s => { s.user.spaces = []; }, s => { s.user.spaces.push({...s.space}); },
    s => { delete s.space.configuration; }, s => { s.space.configuration = null; },
    s => { s.space.configuration.disableAi = 'false'; }]) {
    const value = fixture(); change(value.state);
    await refused(value, 'policy_space');
  }
});

test('page device and engine changes while importing refuse before the getter', async () => {
  for (const change of [s => { s.page += '/changed'; }, s => { s.device += 'changed'; },
    s => { s.engine = {...s.engine}; }]) {
    const value = fixture();
    value.state.duringImport = url => { if (url === POLICY) change(value.state); };
    await refused(value, 'policy_pre_context');
  }
});

test('post-getter binding drift discards even an allowed policy result', async () => {
  for (const change of [s => { s.page += '/changed'; }, s => { s.device += 'changed'; },
    s => { s.engine = {...s.engine}; }, s => { s.store.projectId = 'other'; },
    s => { s.store.project.id = 'changed'; }, s => { s.user.spaces = []; },
    s => { s.space.configuration.disableAi = true; }, s => { s.space.scope = 'user'; }]) {
    const value = fixture(); value.state.duringPolicy = change;
    await refused(value, 'policy_post_context', 1);
  }
});

test('nonboolean policy results stay unknown', async () => {
  for (const result of [undefined, null, 0, 1, 'false', {private: PRIVATE}]) {
    const value = fixture(); value.state.policyResult = result;
    await refused(value, 'policy_result', 1);
  }
});

test('import and policy exceptions export only fixed stage and boolean evidence', async () => {
  const imported = fixture();
  imported.state.duringImport = () => { throw new Error(PRIVATE); };
  await refused(imported, 'policy_import');
  const called = fixture();
  called.state.duringPolicy = () => { throw new Error(PRIVATE); };
  await refused(called, 'policy_check', 1);
});

test('an unset optional flag uses the real policy getter rather than a replacement policy', async () => {
  const value = fixture(); delete value.state.space.configuration.disableAi;
  assert.deepEqual(await value.run(), accepted);
  assert.equal(value.state.calls, 1);
});

test('known user space needs no organization configuration', async () => {
  const value = fixture(); value.state.space.scope = 'user';
  delete value.state.space.configuration;
  assert.deepEqual(await value.run(), accepted);
  assert.equal(value.state.calls, 1);
  assert.deepEqual(value.state.imports, [ENGINE, POLICY]);
});

test('user-space policy never reads inapplicable configuration', async () => {
  const value = fixture(); value.state.space.scope = 'user';
  Object.defineProperty(value.state.space, 'configuration', {
    get() { throw new Error(PRIVATE); },
  });
  assert.deepEqual(await value.run(), accepted);
  assert.equal(value.state.calls, 1);
});

test('missing or unknown space scope stays unknown despite an allowed getter', async () => {
  for (const kind of [undefined, null, '', 'team', false]) {
    const value = fixture(); value.state.space.scope = kind;
    await refused(value, 'policy_space');
  }
});

async function refused(value, stage, calls = 0) {
  const result = await value.run();
  assert.equal(result.stage, stage);
  assert.equal(result.policy_known, false);
  assert.equal('ai_disabled' in result, false);
  assert.equal(value.state.calls, calls);
  assert.equal(value.state.forbidden, 0);
  safe(result);
}

function safe(result) {
  assert.ok(Object.keys(result).every(key => key === 'stage' || keys.includes(key)));
  for (const [key, value] of Object.entries(result))
    if (key !== 'stage') assert.equal(typeof value, 'boolean');
  assert.equal(JSON.stringify(result).includes('private'), false);
}

test('allowed policy uses exact cached module identities and one policy call', async () => {
  const value = fixture();
  const result = await value.run();
  assert.deepEqual(result, accepted);
  assert.deepEqual(value.state.imports, [ENGINE, POLICY]);
  assert.equal(value.state.calls, 1);
  assert.equal(value.state.forbidden, 0);
  safe(result);
});

test('true disableAi returns a fixed refusal without changing the policy', async () => {
  const value = fixture();
  value.state.space.configuration.disableAi = true;
  value.state.policyResult = true;
  const result = await value.run();
  assert.deepEqual(result, {...accepted, stage: 'policy_disabled', ai_disabled: true});
  assert.equal(value.state.space.configuration.disableAi, true);
  assert.equal(value.state.forbidden, 0);
  safe(result);
});

test('progress contains only ordered fixed labels and does not change the result', async () => {
  const value = fixture(); value.state.progressEnabled = true;
  assert.deepEqual(await value.run(), accepted);
  assert.deepEqual(value.state.progress, ['policy_eval_entry', 'policy_engine_import',
    'policy_engine_loaded', 'policy_module_import', 'policy_module_loaded',
    'policy_call', 'policy_evaluated'].map(x => 'fixture:' + x));
});

test('throwing console telemetry cannot prevent a valid policy result', async () => {
  const value = fixture(); value.state.progressEnabled = true; value.state.progressError = true;
  assert.deepEqual(await value.run(), accepted);
  assert.equal(value.state.calls, 1);
});

test('a pending import leaves the corresponding marker without claiming completion', async () => {
  for (const target of [ENGINE, POLICY]) {
    const value = fixture(); value.state.progressEnabled = true;
    let release, reached;
    const gate = new Promise(resolve => { release = resolve; });
    const started = new Promise(resolve => { reached = resolve; });
    value.state.duringImport = async url => { if (url === target) { reached(); await gate; } };
    const pending = value.run();
    await started;
    const stage = target === ENGINE ? 'policy_engine_import' : 'policy_module_import';
    assert.equal(value.state.progress.at(-1), 'fixture:' + stage);
    assert.equal(value.state.calls, 0);
    release();
    assert.deepEqual(await pending, accepted);
  }
});
