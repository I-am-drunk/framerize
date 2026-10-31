# Warm headless Chrome RAM optimization

Scope: `scripts/offline_worker/experiments/service_cdp.mjs` (CDP mint backend).
Measured offline against a simulated mint lifecycle (disposable context ->
target -> attach -> graph bundle eval -> in-page mint -> close) on about:blank.
No live Framer request ever fired; the live service on 8787 was never touched.

## Headline numbers (Chrome 155, `--headless=new`)

| phase | before (base args) | after (patched) | reduction |
|---|---|---|---|
| spawned (idle, before first mint) | 1197.8 MB / 9 procs | 389.6 MB / 3 procs | 3.1x |
| during a mint (page alive) | 1584.5 MB / 11 procs | 771.0 MB / 5 procs | 2.1x |
| steady warm (between mints) | 1107.1 MB / 8 procs | 407.0 MB / 3 procs | 2.7x |

All numbers are per-process VmRSS sums of the Chrome tree, attributed by the
`--type=` flag in each process's cmdline. A mint still returns a genuine
in-page token (token_len ~5000) after every change.

## Honest ceiling

The steady warm footprint is now ~407 MB across 3 processes. That is a true
~2.7x reduction vs the 1107 MB starting warm tree, not the 30-45x the brief
hoped for. The floor is set by three things that cannot be removed without
breaking the mint:

1. The root browser process (~198 MB) holds the graph bundle source and CDP
   session; it cannot shrink below this without dropping devtools.
2. The network service utility (~119 MB) is required because the mint page
   evaluates the SDK graph, which expects network primitives present.
3. The storage service utility (~90 MB) backs the disposable browser context's
   isolated storage the mint depends on.

During a mint two renderers exist (~770 MB total): one runs the mint page
(~180 MB, irreducible because the 5 MB Castle graph bundle must evaluate
somewhere), and one is the `--headless=new` omnibox browser_ui parasite
(~178 MB) that no flag removes (see below).

## Task 1: process map

Baseline warm tree (base args), 8 processes at steady state:

| process | RSS | what it is | needed? |
|---|---|---|---|
| browser | 208 MB | root Chrome process, owns CDP | yes |
| renderer x4 | 580 MB | see below | 1 of 4 |
| gpu-process | 110 MB | GPU helper, unused (gpu disabled) | no |
| utility:network.mojom.NetworkService | 119 MB | network stack | yes |
| utility:storage.mojom.StorageService | 89 MB | context storage | yes |

The four baseline renderers were identified via `Target.getTargets`:
- one `page about:blank` startup tab (the launch URL in `chromeArgs`),
- two `browser_ui chrome://omnibox-popup.top-chrome*` pages,
- one Google Hangouts component-extension background page,
- one component-extension service_worker.

None of these four is a mint renderer. A mint renderer only exists between
`Target.createTarget` and `Target.closeTarget` and was always cleaned up
correctly by `closeMintSession`; the 940 MB figure the brief quoted included
these four squatters plus the gpu-process.

## Task 2: renderer reduction

Changes (all in `scripts/offline_worker/experiments/service_cdp.mjs`):

- `chromeArgs` line 43: added `--disable-component-extensions-with-background-pages`.
  Kills the Hangouts background page and the extension service_worker (2 of
  the 4 squatters, ~270 MB). This flag is invisible to page JS: the mint page
  never sees component extensions.
- New function `closeStartupTargets` lines 67-77, called from `ensureBrowser`
  line 156. Closes the startup `about:blank` page and any `browser_ui` target
  once at browser spawn. The mint path creates its own page inside a disposable
  context, so the startup tab is pure waste (~176 MB renderer).

Result: renderer count at steady state went from 4 to 0. The warm browser now
idles with zero renderers between mints.

The omnibox `browser_ui` pages respawn on demand and share one renderer, so
during a mint there is still one extra ~178 MB renderer beyond the mint page.
It is confirmed parasite: `SystemInfo.getProcessInfo` shows it accumulating
CPU time during the mint's graph eval because `--headless=new` routes the
5 MB bundle through shared V8 resources. No tested flag kills it
(`OmniboxPopup*`, `WebUI*`, `TopChromeWebUI`, `HeadlessNew` toggles,
`--ozone-platform=headless`, field-trial overrides, CDP `Target.closeTarget`
on the browser_ui targets). This is a Chrome 155 `--headless=new` behavior,
not something the mint code controls.

## Task 3: utility reduction

- `chromeArgs` line 40: appended `AudioServiceOutOfProcess` to the existing
  `--disable-features` list. The audio utility process no longer spawns
  (it appeared intermittently in baseline at ~86 MB).
- `chromeArgs` line 46: added `--in-process-gpu`. The gpu-process (~110 MB)
  is folded into the browser process. `--disable-gpu` was already set; this
  just removes the now-pointless helper process. Invisible to Castle because
  the mint never touches GPU.

## Task 4: root/gpu reduction

- `chromeArgs` line 44: added `--disable-site-isolation-trials` and
  `--renderer-process-limit=1`. With one site and a hard cap, Chrome never
  speculatively spawns extra renderers. No Castle-visible effect: the mint
  page is same-site with about:blank and never needs isolation.
- `chromeArgs` line 45: added `--enable-features=MemoryPurgeOnBackground`.
  Chrome returns freed renderer memory to the OS between mints. No page-visible
  effect.
- The gpu-process is gone via `--in-process-gpu` (task 3). Root browser RSS
  stays ~198 MB because it hosts the CDP session and the graph bundle string;
  that is the cost of the CDP backend itself.

Flags tried and rejected because they changed nothing measurable:
`--js-flags=--max-old-space-size=96/64/32`, `--memory-reducer`,
`--single-threaded`, `--no-sparkplug --no-maglev --no-turbofan`,
`--process-per-tab`, `--freeze-dried-tab-mode`, `MemoryPurgeOnBackgroundAggressive`,
`--disable-features=StorageService` (storage utility is required),
`--disable-features=NetworkService` (breaks mint), `--single-process`
(devtools never comes up).

## Task 5: verification

Every variant above was run through the offline harness
(`experiments/ramopt/ram_measure.mjs`) which mirrors the exact session
lifecycle of `service_cdp.mjs`. The mint still returned a genuine in-page
token (token_len ~5000) after every accepted flag and after the final patch.
The final patched configuration was run 4 mints back-to-back: steady state
remained 407 MB with no renderer or utility accumulation, confirming mints
do not leak processes.

Files changed:
- `scripts/offline_worker/experiments/service_cdp.mjs` lines 40, 43-46
  (chromeArgs) and lines 67-77, 156 (closeStartupTargets + call site).

Measurement harness (not part of the service):
- `scripts/offline_worker/experiments/ramopt/ram_measure.mjs`
- `scripts/offline_worker/experiments/ramopt/RAMOPT_REPORT.md` (this file)
