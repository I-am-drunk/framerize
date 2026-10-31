// Offline RAM harness for the warm CDP mint backend.
// Spawns Chrome with a selectable arg set, runs simulated mint cycles that
// mirror service_cdp.mjs's session lifecycle (context -> target -> attach ->
// eval graph bundle -> trace replay -> in-page mint -> close), and reports
// per-process RSS of the whole tree. NEVER navigates to framer.com and never
// touches the live service: the mint page is about:blank. The graph bundle
// mint runs fully in-page, so the token is still genuinely computed.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import * as pins from '../../pins.mjs';

const CHROME = '/home/tjninja/.local/share/lets-study-test-browser/chrome/opt/google/chrome/chrome';
const PORT = 9700 + (process.pid % 200);
const PROFILE = '/tmp/ramopt-profile-' + process.pid;

function get(port, p) {
  return new Promise((res, rej) => {
    http.get({ host: '127.0.0.1', port, path: p }, r => {
      let b = ''; r.on('data', c => b += c);
      r.on('end', () => { try { res(JSON.parse(b)); } catch (e) { rej(e); } });
    }).on('error', rej);
  });
}
const sleep = ms => new Promise(r => setTimeout(r, ms));

// Arg sets. base = current production chromeArgs from service_cdp.mjs.
const BASE_ARGS = [
  '--headless=new', '--no-first-run', '--no-default-browser-check',
  '--disable-extensions', '--disable-sync', '--disable-background-networking',
  '--disable-component-update', '--mute-audio', '--no-sandbox',
  '--window-size=1280,800', '--force-device-scale-factor=1',
  '--disable-gpu', '--disable-software-rasterizer', '--disable-dev-shm-usage',
  '--disable-features=Translate,BackForwardCache,AcceptCHFrame,MediaRouter,OptimizationHints,InterestFeedContentSuggestions,CalculateNativeWinOcclusion',
  '--disable-breakpad', '--disable-crash-reporter', '--disable-logging', '--no-zygote',
  '--disable-renderer-backgrounding', '--disable-background-timer-throttling',
  '--js-flags=--max-old-space-size=128',
  '--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36',
];

// Candidate additions, each one must be justified by a measured delta and by
// not changing what Castle observes (all are invisible to page JS).
const EXTRA = {
  // Kill the component extensions (Hangouts background page etc) that Chrome
  // force-loads even with --disable-extensions, and the omnibox browser-UI
  // pages. Both spawn renderers that a mint never touches.
  'no-component-ext': ['--disable-component-extensions-with-background-pages'],
  // Kill utility processes a headless minter never uses.
  'no-audio-service': ['--disable-features=AudioServiceOutOfProcess'],
  'no-speech': ['--disable-speech-api', '--disable-speech-synthesis-api'],
  // Renderer/site policy: no isolation, cap renderer count, no preloaded spare.
  'no-isolation': ['--disable-site-isolation-trials'],
  'renderer-limit': ['--renderer-process-limit=1'],
  // In-engine memory reclamation; no page-visible behavior change.
  'mem-savings': ['--enable-features=MemoryPurgeOnBackground'],
  // Non-headless pieces the minter never uses; none are page-visible.
  'no-apps': ['--disable-features=DesktopPWAsRunOnOsLogin,WebAppInstallation'],
  'no-domain-reliability': ['--disable-domain-reliability'],
  'no-omnibox-ui': ['--disable-features=OmniboxPopupWebUI'],
  'no-top-chrome': ['--disable-features=WebUITopChrome'],
  'old-headless': ['--headless=old'],
  'headless-v1': ['--headless=chrome'],
  'no-startup-tab': [], // marker: drop 'about:blank' from launch args
  'in-proc-gpu': ['--in-process-gpu'],
  'disable-lcd': ['--disable-lcd-text'],
  'canvas-oop-off': ['--disable-accelerated-2d-canvas'],
  'single-raster': ['--num-raster-threads=1'],
  'no-mojo-media': ['--disable-features=MojoVideoCapture,MediaSessionService'],
  'aggressive-mem': ['--enable-features=MemoryPurgeOnBackground,MemoryPressureReduction,RendererCodeIntegrity'],
  'low-mem-mode': ['--enable-low-end-device-mode'],
  'force-purge': ['--enable-purge-and-suspend'],
  'no-site-db': ['--disable-features=SitePerProcessOnlyUseHttpsABitMore'],
  'js64': ['--js-flags=--max-old-space-size=64'],
  'js32': ['--js-flags=--max-old-space-size=32'],
  'old-space-192': [],
  'cache-code-off': ['--no-v8-cache'],
  'no-preparse': ['--js-flags=--no-lazy'],
  'lazy-off': ['--js-flags=--no-compile-hints'],
  'single-process-inproc-gpu': ['--single-process-in-process-gpu'],
  'old-headless-feat': ['--disable-features=HeadlessNew'],
  'no-shared-img': ['--disable-features=SharedImageManager'],
  'deterministic': ['--deterministic-mode'],
  'force-prefers-reduced': ['--force-prefers-reduced-motion'],
  'plain-headless': ['--headless'],
  'disable-browser-ui': ['--disable-features=WebUIContents'],
  'new-headless-pure': ['--enable-features=HeadlessNew'],
  'k-headless-check': ['--headless=new', '--disable-features=OmniboxPopup'],
  'no-warmup': ['--no-startup-window'],
  'remote-allow-all': ['--remote-allow-origins=*'],
  'webui-off': ['--disable-features=WebUI'],
  'chrome-urls-off': ['--disable-chrome-google-url-tracking-client'],
  'headless-mode-old': ['--headless'],  // plain --headless maps to old headless in 155 if still supported
  'disable-omnibox-aim': ['--disable-features=OmniboxAIM,OmniboxAimPopup,AimEligibilityService'],
  'omnibox-kill': ['--disable-features=OmniboxPopup,OmniboxPopupWebUI,WebUIOmniboxPopup,OmniboxPopupContents'],
  'toolkit-views-off': ['--disable-features=ToolkitViews'],
  'browser-ui-off': ['--disable-browser-ui'],
  'kiosk': ['--kiosk'],
  'new-base': [],  // alias marker
  'native-headless': [],  // placeholder
  // Trim utility process: no file access beyond the throwaway profile, no
  // background fetch, no font service in headless.
  'no-font-service': ['--disable-features=FontService'],
  'no-ml': ['--disable-features=OptimizationGuideModelDownloading,OptimizationHints,OnDeviceModel'],
  'no-webrtc': ['--disable-features=WebRtcHideLocalIpsWithMdns'],
  'no-speech-full': ['--disable-speech-api', '--disable-speech-synthesis-api', '--disable-features=SpeechRecognition,SpeechSynthesis'],
  'kill-webui': ['--disable-features=WebUI,WebUIContents,WebUIBrowserInterface,TopChromeWebUI,WebUiMojoAbout'],
  'k-webui-popup': ['--disable-features=OmniboxPopupWebUI,WebUIPopup'],
  'hard-kill-ui': ['--disable-features=WebUI,WebUIContents,WebUIBrowserInterface,TopChromeWebUI,OmniboxPopup,OmniboxPopupWebUI,WebUiMojoAbout,WebUIOmniboxPopup'],
  'no-codecache': ['--disable-features=V8CodeCache,V8CodeCacheOnPageHide'],
  'v8-lite': ['--js-flags=--lite-mode'],
  'no-preload': ['--disable-features=PreloadEngagement,PreloadingNoDryRun'],
  'no-network-predict': ['--disable-network-action-predictor'],
  'no-spare-renderer': ['--disable-features=SpareRendererForSitePerProcess'],
  'low-priority-renderer': ['--renderer-process-priority=low'],
  'force-fieldtrial': ['--force-fieldtrials=WebUI/Disabled/'],
  'no-profile-ctx': [],  // marker: create targets in default context
  'warmup-close': [],    // marker: warmup then close the browser_ui renderers
  'browser-mem-srv': ['--disable-features=BrowserMemoryService'],
  'no-discard': ['--disable-features=TabDiscarding,TabFreeze'],
  'eco-mode': ['--enable-features=EcoMode'],
  'freeze-dried': ['--freeze-dried-tab-mode'],
  'tab-freeze': ['--enable-features=FreezeBackgroundTabs'],
  'bg-renderer-low': ['--background-renderer-process-priority=low'],
  'k-headless': ['--k-headless'],
  'x11-off': ['--disable-x11'],
  'ozone-headless': ['--ozone-platform=headless'],
  'in-proc-storage': ['--disable-features=StorageService'],
  'in-proc-all-utils': ['--disable-features=StorageService,AudioServiceOutOfProcess'],
  'renderer-idle': ['--disable-features=RendererIdleTasks'],
  'no-throttle-bg': ['--disable-background-media-suspend'],
  'mem-ablation': ['--mem-pressure-system-reserved-kb=1'],
  'purge-throttle': ['--enable-features=MemoryPurgeOnBackground,MemoryPurgeOnBackgroundAggressive'],
  'v8-mem-reducer': ['--js-flags=--memory-reducer'],
  'v8-mem-red+96': [],
  'force-ui-off': ['--disable-features=BrowserRootView,DesktopPWAsTabStrip'],
  'disable-renderers-limit': ['--disable-features=RendererProcessLimit'],
  'truly-min': ['--disable-features=ChromeWhatsNewUI,ChromeSurvey,AutofillEnableAccountWalletStorage,PaymentSettings,PasswordManager,PasswordManagerRedesign,PasswordChange,WebOTP,IdentityInCanMakePaymentEventFeature,GooglePayDynamicUpdate'],
  'proc-per-tab': ['--process-per-tab'],
  'single-renderer-inst': ['--process-per-site'],
  'no-preconnect': ['--disable-preconnect'],
  'minimal-features': ['--disable-features=InterestFeedContentSuggestions,MediaRouter,OptimizationHints,AutofillServerCommunication,AutofillEnableAccountWalletStorage,WebOTP,NotificationTriggers,PrivacySandboxSettings4,SubresourceFilter,SafeBrowsingEnhancedProtection,CertificateTransparencyComponentUpdater,FileSystemAccessPersistentPermissions'],
  'v8-tight': ['--js-flags=--max-old-space-size=96'],
  'in-process-net': ['--disable-features=NetworkService,NetworkServiceInProcess'],
  'single-process': ['--single-process'],
};
const CLOSE_STARTUP = '__CLOSE_STARTUP__';

function buildArgs(variant) {
  let args = [...BASE_ARGS];
  if (variant && variant !== 'base') {
    for (const name of variant.split('+')) {
      if (name === 'close-startup' || name.startsWith('jsf:')) continue;
      const add = EXTRA[name];
      if (!add) throw new Error('unknown variant part: ' + name);
      args = args.concat(add);
    }
  }
  args = args.concat([
    '--remote-debugging-port=' + PORT,
    '--user-data-dir=' + PROFILE,
  ]);
  if (!(variant && variant.split('+').includes('no-startup-tab'))) args.push('about:blank');
  // --headless=new is in BASE_ARGS; an old-headless variant replaces it.
  if (variant && variant.split('+').some(v => v === 'old-headless' || v === 'headless-v1' || v === 'plain-headless')) {
    args = args.filter(a => a !== '--headless=new');
  }
  // A variant js-flags replaces the base one.
  const customJs = (variant || '').split('+').find(v => v.startsWith('jsf:'));
  if (variant && variant.split('+').includes('v8-lite')) {
    args = args.filter(a => !a.startsWith('--js-flags='));
    args.push('--js-flags=--lite-mode');
  }
  if (customJs) {
    args = args.filter(a => !a.startsWith('--js-flags='));
    args.push('--js-flags=' + customJs.slice(4).replace(/_/g, ' '));
  }
  if (variant && (variant.split('+').includes('v8-tight') || variant.split('+').includes('js64') || variant.split('+').includes('js32'))) {
    args = args.filter(a => !a.startsWith('--js-flags='));
    if (variant.split('+').includes('js32')) args.push('--js-flags=--max-old-space-size=32');
    else args.push(variant.split('+').includes('js64') ? '--js-flags=--max-old-space-size=64' : '--js-flags=--max-old-space-size=96');
  }
  return args;
}

async function spawnBrowser(variant) {
  try { fs.rmSync(PROFILE, { recursive: true, force: true }); } catch {}
  const child = spawn(CHROME, buildArgs(variant),
    { stdio: ['ignore', 'ignore', 'ignore'], detached: true });
  for (let i = 0; i < 80; i++) {
    try {
      const v = await get(PORT, '/json/version');
      if (v && v.Browser) {
        const { default: WebSocket } = await import('../../inputs/dom/node_modules/ws/wrapper.mjs');
        const ws = new WebSocket(v.webSocketDebuggerUrl, { perMessageDeflate: false, maxPayload: 64 * 1024 * 1024 });
        await new Promise((res, rej) => { ws.on('open', res); ws.on('error', rej); });
        return { child, ws, nextId: 0, pending: new Map(), sessionListeners: new Map() };
      }
    } catch {}
    await sleep(250);
  }
  throw new Error('devtools not ready');
}

function wire(b) {
  b.ws.on('message', d => {
    let m; try { m = JSON.parse(d); } catch { return; }
    if (m.id && b.pending.has(m.id)) { b.pending.get(m.id)(m); b.pending.delete(m.id); return; }
    if (m.method && m.sessionId && b.sessionListeners.has(m.sessionId)) {
      for (const fn of b.sessionListeners.get(m.sessionId)) { try { fn(m); } catch {} }
    }
  });
  b.ws.on('error', () => {});
}
function bsend(b, method, params = {}) {
  return new Promise((res, rej) => {
    const id = ++b.nextId;
    b.pending.set(id, m => m.error ? rej(new Error(method + ': ' + JSON.stringify(m.error))) : res(m.result));
    b.ws.send(JSON.stringify({ id, method, params }));
  });
}
function ssend(b, s, method, params = {}) {
  return new Promise((res, rej) => {
    const id = ++b.nextId;
    b.pending.set(id, m => m.error ? rej(new Error(method + ': ' + JSON.stringify(m.error))) : res(m.result));
    b.ws.send(JSON.stringify({ id, method, params, sessionId: s.sessionId }));
  });
}
async function seval(b, s, expr, timeout = 30000) {
  const r = await ssend(b, s, 'Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true, timeout });
  if (r.exceptionDetails) throw new Error('eval: ' + JSON.stringify(r.exceptionDetails).slice(0, 300));
  return r.result ? r.result.value : undefined;
}

// Per-process RSS snapshot of the whole chrome tree, keyed by pid, with the
// chrome --type= flag so each row is attributable.
// Find every process whose cmdline carries this run's --user-data-dir (the
// spawned wrapper re-execs, so child-pid tracking misses the real tree).
function treeSnapshot(rootPid, profile) {
  const out = {};
  const tag = profile || PROFILE;
  for (const name of fs.readdirSync('/proc')) {
    if (!/^\d+$/.test(name)) continue;
    try {
      const cmdline = fs.readFileSync('/proc/' + name + '/cmdline', 'utf8').replace(/\0/g, ' ');
      if (!cmdline.includes(tag)) continue;
      const m = cmdline.match(/--type=([a-z-]+)/);
      let type = m ? m[1] : (cmdline.includes('crashpad') ? 'crashpad' : 'browser');
      if (type === 'utility') {
        const svc = cmdline.match(/--utility-sub-type=([a-z0-9_.-]+)/i);
        if (svc) type = 'utility:' + svc[1];
      }
      const status = fs.readFileSync('/proc/' + name + '/status', 'utf8');
      const rss = parseInt((status.match(/VmRSS:\s+(\d+) kB/) || [])[1] || '0', 10);
      out[name] = { type, rss_kb: rss };
    } catch {}
  }
  return out;
}

function totals(snap) {
  const byType = {};
  for (const p of Object.values(snap)) {
    byType[p.type] = (byType[p.type] || 0) + p.rss_kb;
  }
  const total = Object.values(snap).reduce((a, p) => a + p.rss_kb, 0);
  return { total_kb: total, by_type: byType, count: Object.keys(snap).length };
}

// One simulated mint: exactly the session lifecycle of service_cdp.mjs
// (context + target + attach + graph bundle + mint), but on about:blank so
// no live Framer request ever fires. Returns the in-page mint result.
// onMidMint, when supplied, is awaited after the graph loads while the mint
// page is alive, so the caller can snapshot the during-mint tree.
async function simulatedMint(b, { navigate = false, closeSession = true, onMidMint = null, onPhase = null, pageUrl = 'about:blank' } = {}) {
  const { browserContextId } = await bsend(b, 'Target.createBrowserContext', { disposeOnDetach: true });
  const { targetId } = await bsend(b, 'Target.createTarget', { url: pageUrl, browserContextId });
  if (onPhase) await onPhase('after_create_target');
  const { sessionId } = await bsend(b, 'Target.attachToTarget', { targetId, flatten: true });
  const s = { browserContextId, targetId, sessionId };
  b.sessionListeners.set(sessionId, new Map());
  try {
    await ssend(b, s, 'Page.enable', {});
    await ssend(b, s, 'Runtime.enable', {});
    if (process.env.RAMOPT_KILL_BROWSERUI === '1') {
      try {
        const { targetInfos } = await bsend(b, 'Target.getTargets');
        for (const t of targetInfos) {
          if (t.type === 'browser_ui') { await bsend(b, 'Target.closeTarget', { targetId: t.targetId }).catch(() => {}); }
        }
      } catch {}
    }
    if (onPhase) await onPhase('after_attach');
    if (navigate) {
      // Offline substitute for the framer.com navigation: a same-shape local
      // document. Never a live request.
      await ssend(b, s, 'Page.navigate', { url: 'about:blank' });
    }
    if (process.env.RAMOPT_SKIP_GRAPH === '1') {
      if (onPhase) await onPhase('after_graph_loaded');
      if (onMidMint) await onMidMint();
      return { ok: true, tokenLen: 0, skipped: true };
    }
    const { buildGraphScript } = await import('../graph_script.mjs');
    const bundle = buildGraphScript({
      assets: pins.ASSETS, modules: pins.MODULES, cdn: pins.CDN,
      collectionTransform: null,
    });
    await seval(b, s, `(0,eval)(${JSON.stringify(bundle)}); 'graph-loaded'`);
    if (onPhase) await onPhase('after_graph_loaded');
    await seval(b, s, `window.castlePublishableAPIKey = ${JSON.stringify(pins.PUBLIC_KEY)}; 'key-set'`);
    if (onMidMint) await onMidMint();
    const out = await seval(b, s, `(async () => {
      const m = globalThis.__modules.wrapper; if (!m || !m.a) return { err: 'no wrapper' };
      try { const tok = await m.a({ budget_ms: 8000 }); return { tok }; }
      catch (e) { return { err: String(e && e.message || e) }; }
    })()`, 20000);
    if (!out || !out.tok) return { ok: false, err: out && out.err };
    return { ok: true, tokenLen: out.tok.length };
  } finally {
    if (closeSession) {
      b.sessionListeners.delete(sessionId);
      try { await bsend(b, 'Target.detachFromTarget', { sessionId }); } catch {}
      try { await bsend(b, 'Target.closeTarget', { targetId }); } catch {}
      try { await bsend(b, 'Target.disposeBrowserContext', { browserContextId }); } catch {}
    }
  }
}

async function main() {
  const variant = process.argv[2] || 'base';
  const mints = parseInt(process.argv[3] || '3', 10);
  const HOLD = process.env.RAMOPT_HOLD_MS ? parseInt(process.env.RAMOPT_HOLD_MS, 10) : 0;
  const b = await spawnBrowser(variant);
  wire(b);
  if (process.env.RAMOPT_CLOSE_STARTUP_TAB === '1' || variant.split('+').includes('close-startup')) {
    try {
      const { targetInfos } = await bsend(b, 'Target.getTargets');
      for (const t of targetInfos) {
        if (t.type === 'page' || t.type === 'browser_ui') { await bsend(b, 'Target.closeTarget', { targetId: t.targetId }).catch(() => {}); }
      }
    } catch {}
  }
  await sleep(1500); // let the tree settle to warm idle

  const phases = {};
  phases.spawned = treeSnapshot(b.child.pid, PROFILE);

  // Warm the browser the way the first mint does; snapshot while page is live.
  let during = null;
  let duringTargets = null;
  let duringProcInfo = null;
  const subphases = {};
  const PAGE_URL = process.env.RAMOPT_PAGE_URL || 'about:blank';

  const r1 = await simulatedMint(b, { pageUrl: PAGE_URL, onPhase: async (name) => {
    subphases[name] = treeSnapshot(b.child.pid, PROFILE);
  }, onMidMint: async () => {
    during = treeSnapshot(b.child.pid, PROFILE);
    try {
      const infos = (await bsend(b, 'Target.getTargets')).targetInfos;
      duringTargets = [];
      for (const t of infos) {
        const rec = { type: t.type, url: t.url, targetId: t.targetId };
        try {
          const { processInfo } = await bsend(b, 'SystemInfo.getProcessInfo');
          void processInfo;
        } catch {}
        duringTargets.push(rec);
      }
      try {
        const { processInfo } = await bsend(b, 'SystemInfo.getProcessInfo');
        duringProcInfo = processInfo;
      } catch (e) { duringProcInfo = String(e && e.message); }
    } catch {}
  } });
  phases.during_mint1 = during;
  phases.subphases = subphases;
  await sleep(2000);
  phases.after_mint1 = treeSnapshot(b.child.pid, PROFILE);

  let last = r1;
  for (let i = 1; i < mints; i++) {
    last = await simulatedMint(b);
    if (process.env.RAMOPT_PURGE === '1') { try { await bsend(b, 'Memory.purgeMemory'); } catch {} }
    await sleep(1500);
  }
  phases.steady = treeSnapshot(b.child.pid, PROFILE);

  // Map live targets to urls for renderer attribution.
  let targets = [];
  try { targets = (await bsend(b, 'Target.getTargets')).targetInfos; } catch (e) { targets = [{ err: e.message }]; }
  let targetList = [];
  try { targetList = await get(PORT, '/json/list'); } catch {}

  // Report.
  const report = {
    targets, targetList, duringTargets, duringProcInfo,
    variant,
    mints_ok: [r1, last].every(r => r.ok),
    last_token_len: last.tokenLen || 0,
    last_err: last.err || null,
    phases: Object.fromEntries(Object.entries(phases).map(([k, v]) =>
      [k, (k === 'subphases')
        ? Object.fromEntries(Object.entries(v).map(([sk, sv]) => [sk, sv ? { ...totals(sv), procs: sv } : null]))
        : { ...totals(v), procs: v }])),
  };
  console.log(JSON.stringify(report, null, 1));
  if (HOLD > 0) {
    console.error('HOLDING', HOLD, 'ms; tree pid root', b.child.pid, 'profile', PROFILE);
    await sleep(HOLD);
  }

  try { b.ws.close(); } catch {}
  try { process.kill(-b.child.pid, 'SIGKILL'); } catch {}
  try { fs.rmSync(PROFILE, { recursive: true, force: true }); } catch {}
  process.exit(0);
}

main().catch(e => { console.error('FATAL', e.message); process.exit(1); });
