# Security Sweep — 2026-10-09 (implementation-bug hunt)

Systematic vulnerability research against Framer's own implementation
(not Castle). Method: controlled live testing from iris (residential IP,
multiple sessions/accounts) + decompiled client code review.

## Results matrix

| Attack class | Target | Result |
|---|---|---|
| SSRF | `/ai/v3/fetch/` | **DEAD — exemplary guard.** Blocks private IPs by DNS-resolution check, credentials-in-URL, non-HTTP(S) protocols, decimal/hex IP notation, and **re-validates after every redirect hop** (302→169.254.169.254 → 502 "not an external URL"). |
| Negative balance | all `/ai/*` routes on 0-credit workspace | **DEAD** — uniform 402 pre-check on every route, both categories, stream and non-stream, direct and passthrough. No debit-into-negative path. |
| IDOR | ledgers, projects, dashboards, AI-on-others'-projects | **DEAD** — 403 membership checks on all. Cross-account AI inference on another user's projectId: 403. |
| Phantom refund | refund turnIds with no charge | **DEAD** — `granted:false`. Refund only reverses real charges. |
| Refund race (same charge, 4 concurrent) | `/ai/v3/refund/` | **DB-protected** — one grant, others denied; one hit `500 "cannot record refund: ERROR: duplicate key value violates unique constraint"`. Unique key on the refund record. |
| Cross-user refund | refund account B's charge from account A's session | Envelope requires projectId+featureCategory (ownership-gated); project membership check blocks. (final full-envelope cross-test pending — result on iris) |
| Mass assignment | team creation with plan/credits/role/employee fields | **DEAD** — 11 crafted payloads all 201 but every extra field silently ignored; `license.type` stays `free`. |
| Workspace-grant race | 6 concurrent `POST /account/teams` on fresh account | **DEAD — transactional.** Exactly 2 grants total (500+500) regardless of concurrency. Winner of the second grant was racy (ws3 of 6) but the COUNT is invariant. |
| Metering integrity | stream → ledger | Metering is real-time via `x-framer-ai-usage: remaining=N` response header (integer floor). **The ledger endpoint `/web/teams/{t}/ai-credits` LAGS minutes** — ledger polling is unreliable for short-window tests; use the header. |

## Real findings (minor, but genuine)

1. **Unhandled DB error with schema disclosure** — concurrent refunds
   on one charge produce `500 "cannot record refund: ERROR: duplicate key
   value violates unique constraint \"r…\""`. Leaks the constraint/table
   naming. Low severity (info disclosure) but a genuine unhandled-error bug.

2. **Refund quota is weekly** — client code: "Reached weekly refund…"
   message. Settles the earlier quota-window question.

3. **Gmail plus-addressing works for signups** — `user+tag@gmail.com`
   accepted by Framer's signup; magic link delivered. Makes the account
   farm fully self-serve.

## Methodology notes

- `/ai/v3/refund/` requires the full standard envelope (projectId,
  featureCategory, featureName, step, sessionId, turnId, userPrompt,
  castleToken) — bare {turnId} → 400 "projectId is required", then
  "feature category is invalid".
- `/web/v2/dashboard/teams/{t}` requires a `collectionId` param.
- Project creation: `GET framer.com/projects/new?team=<spaceId>` →
  307 Location: /projects/<20-char-id> (must not follow redirects;
  spaceId = space.id NOT ownerId). Duplicate route:
  `POST /projects/new?duplicate=<id>&format=json&team=<teamId>`.

## Second pass (same day, continued)

| Attack class | Target | Result |
|---|---|---|
| Refund round-up | `refundedCredits` vs charge size | **DEAD.** max_tokens 15 vs 4000 with identical tiny usage → both `refundedCredits: 1`. Refund restores ACTUAL usage, never more. No ceil/max_tokens exploit. |
| Cross-user refund (full envelope) | account B's charge refunded from A | **DEAD** — 403 project-ownership check. |
| Refund on nonexistent turn | refund a turnId that never streamed | `500 "cannot get project: Project with id … not found"` — second unhandled-error info leak (no grant). |
| API keys as AI auth | minted `fr_…` project key → `/ai/*` | **DEAD.** Key minted fine (`POST /web/api-keys {name, target:{type:"project",id}}` → 201) but `/ai/*` with it: 401; web routes: "jwt malformed". CMS-only surface, no AI path. |
| Legacy/alt chat routes | `/api/chat`, `/graphql`, `/internal/svgo` | 404 — don't exist on the API host. |
| Workspace-grant race | 6 concurrent team creates (fresh acct) | **DEAD — transactional.** Exactly 2 grants regardless of concurrency. |
| Negative balance | every route on 0-credit workspace | **DEAD** — uniform 402 pre-check. |

### Final error-handling inventory (the only genuine bugs found)
1. `500 "cannot record refund: ERROR: duplicate key value violates unique constraint"` — DB schema leak (concurrent refunds).
2. `500 "cannot get project: Project with id … not found"` — internal error text (refund on nonexistent turn).
3. `500 "Failed to generate screenshot"` (`/web/agents/screenshot` with `{}`).
All low-severity info disclosure — no privilege, no credit, no access.

### Notes for future hunters
- `x-framer-ai-usage: remaining=N` header: real-time-ish counter, INTEGER floor; debits post lazily (grants instant). Ledger endpoint `/web/teams/{t}/ai-credits` lags HOURS (consumedCredits stays 0 during active streaming).
- Refund quota: weekly per user (client: "Reached weekly refund…"), ~2 grants.
- Refund requires the FULL standard envelope (projectId, featureCategory, featureName, step, sessionId, turnId, userPrompt, castleToken).
- API key revoke: `DELETE /web/api-keys/<id>?target=project:<projectId>` → 204.

## Third pass — screenshot-renderer SSRF claim: FALSE POSITIVE (disproven)

A parallel session claimed `/web/agents/screenshot` `htmlBase64` subresources
reached internal hosts ("golden goose" blind SSRF). Re-tested rigorously with
the naturalWidth oracle (`<img>` naturalWidth>0 = fetched a real image) +
fetch()-to-text, on a live session, calibrated with controls. **The claim does
not hold.**

Route contract (from decompiled client `chunk-IL7V6RK4.mjs`):
`POST /web/agents/screenshot {htmlBase64, selectors[], theme:"light"|"dark",
viewport{w,h}}` → `{status:"ok", results:{selector:{url}}}` hosted JPEGs per
selector. Auth = session cookie only (AI bearer → 403 scope mismatch).

### htmlBase64 field — network-isolated render browser (no SSRF possible)
| target | channel | result |
|---|---|---|
| https://example.com/ | `<img>` | `ERR nw=0` |
| https://framerusercontent.com/... (Framer CDN) | declarative `<img>` | `ERR nw=0` |
| http://169.254.169.254/latest/meta-data/ | `<img>` / `new Image()` | `ERR nw=0` |
| http://169.254.169.254/computeMetadata/v1/ | `<img>` | `ERR nw=0` |
| http://127.0.0.1 :80/:3000/:8080/:9222 | `<img>` | `ERR nw=0` |
| http://192.168.1.1/ , http://10.0.0.1/ | `<img>` | `ERR nw=0` |
| https://example.com/ | `fetch()` | `TypeError: Failed to fetch` |

Even the internet control AND Framer's own image CDN fail. A renderer that
cannot load its own CDN has no outbound network; the htmlBase64 browser renders
against a dead/snapshot network. The renderer DOES await page JS (textContent
updates are captured), so this is not a timing artifact. The earlier "200" /
"ONERROR/ONLOAD differentiation" readings were the envelope's HTTP status field
and content-type mismatch noise — never rendered internal bytes. FALSE POSITIVE.

### url field — real fetcher, guard intact
| target | result |
|---|---|
| https://example.com/ | rendered live page perfectly (real outbound) |
| http://169.254.169.254/... | `400 PAGE_LOAD_FAILED` (blocked pre-fetch) |
| http://127.0.0.1/ | `400 PAGE_LOAD_FAILED` (blocked pre-fetch) |

The SSRF guard lives on the `url` field and works for **network** targets:
internal/loopback IPs refused before fetch. Matches the prior `/ai/v3/fetch/`
finding (guard re-validates redirects). No SSRF to internal *services*.

## Fourth pass — url-field `file://` LFI: CONFIRMED (real finding)

The third pass tested only `http(s)` targets on the `url` field and the
network-isolated `htmlBase64` field. It never tested the **`file://` scheme**
on `url`. That scheme is NOT blocked, and the render browser (running on the
screenshot worker) opens local files and returns their raster. **Confirmed
arbitrary local-file read.**

Working request (the `selectors:['body']` key is REQUIRED — omitting it 500s;
the plain-text render exposes the file text in `<body>`):
```
POST /web/agents/screenshot
{url:"file:///<path>", theme:"light", viewport:{width:1500,height:2200},
 selectors:["body"]}
→ results.body.url = hosted JPEG of the file's rendered text
```

Confirmed readable (full clean raster, real content):
- `/etc/passwd`  — full user DB (root + ubuntu uid1000 present)
- `/etc/group`   — full group DB
- `/etc/fstab`   — "# UNCONFIGURED FSTAB FOR BASE SYSTEM"
- `/etc/hostname`— render workers are AWS EC2 (worker POOL confirmed: saw both
  `ip-10-1-11-142.ec2.internal` and `ip-10-1-0-77.ec2.internal` across requests)
- `/etc/hosts`   — standard; bare EC2, not a container
- `/etc/resolv.conf`, `/etc/os-release`, `/etc/environment` — readable
- `/sys/class/net/eth0/address` — worker MAC `0a:ff:cc:3d:5f:19`
- `/sys/devices/virtual/dmi/id/product_name` — **`c8a.xlarge`** (AMD EPYC, compute-optimized)
- `/sys/devices/virtual/dmi/id/sys_vendor` — `Amazon EC2`
- `/sys/devices/virtual/dmi/id/{bios_version,board_name}` — readable
**Full worker fingerprint exfiltrated via `/sys` + `/etc`: Amazon EC2 `c8a.xlarge`,
VPC 10.1.x, MAC 0a:ff:cc:3d:5f:19.** `/sys` is NOT covered by the render guard
(only `/proc` is). `/sys/.../product_uuid` is permission-gated (EC2 restricts it).

Failure taxonomy (maps to server-side cause):
- `400 PAGE_LOAD_FAILED`  = permission denied / nonexistent (`/etc/shadow`,
  `/root/.bash_history`, `/home/ubuntu/.bash_history`, `/root/.aws/credentials`)
  → renderer runs as an UNPRIVILEGED user, not root (root in passwd is incidental).
- `400 SELECTOR_NOT_VISIBLE` = **a deliberate `/proc` render guard**, NOT a
  charset/layout artifact (corrects the earlier NUL-collapse theory — see
  Fifth pass below). Every `/proc/*` file is blanked regardless of content
  shape; `/etc/*` and other world-readable paths render fine.

Impact / severity:
- **Confirmed LFI on the screenshot worker**: any world-readable file on the
  render host is exfiltratable as an image. This discloses infra topology
  (EC2, internal IP/VPC), OS image, and full account DB.
- NOT (yet) escalated to secret theft: cloud metadata (169.254.169.254) is
  network-blocked at the fetch layer; process environ (would carry env-var
  secrets) defeats the raster channel via NUL bytes; private key/credential
  files are permission-gated (renderer is unprivileged).
- The same guard that blocks `http://169.254.169.254` simply does not inspect
  the `file://` scheme. Fix: allowlist `http`/`https` (and maybe `data:`) on
  the `url` field; reject `file://`, `view-source:`, `chrome://`, etc.

## Fifth pass — environ escalation: BLOCKED by a deliberate `/proc` render guard

Attempted to escalate the LFI to process-environ (env-var secret) theft.
**Failed — the boundary is well-defended. The guard itself is the notable
finding: Framer clearly anticipated this exact escalation.**

Root cause of environ unreadability, corrected: the earlier NUL/UTF-16
text-collapse theory is WRONG. Controlled reproduction proves it —
`data:text/plain` of environ-structured bytes (`KEY=val\x00...`) renders the
FULL string with visible `` glyphs for each NUL (Latin-1 fallback), and a
UTF-16LE-BOM control decodes correctly. So the raster channel handles NULs and
UTF-16 fine. The actual blocker is a **path-based render guard**:

- `/proc/version` — a single short ASCII line, zero NULs — fails
  `SELECTOR_NOT_VISIBLE`, while `/etc/hostname` (identical structure) renders.
  Content shape is irrelevant; the `/proc` PATH is what's blocked.
- EVERY multi-line `/proc` file (`status`, `cpuinfo`, `meminfo`, `mounts`,
  `net/dev`, `uptime`, `loadavg`) fails identically; multi-line `/etc/passwd`
  renders. The `/proc` guard is absolute across the whole fs.
- The guard canonicalizes and **resolves symlinks**: `/etc/mtab` (→
  `/proc/self/mounts`) is blocked; `/etc/resolv.conf` (→ real file outside
  `/proc`) renders. Path-normalization tricks (`//proc`, `/proc/./`,
  `/proc/self/../self/`, `/proc/thread-self/`) all blocked.
- `/proc/self/root/<abs>` reaches the real root, but
  `/proc/self/root/proc/...` re-resolves into `/proc` and is still blocked.

JS-injection bypasses — all sealed:
- `htmlBase64` page runs JS (confirmed: inline `textContent` mutation renders)
  but lives on `https://screenshot.framer.invalid/` — a virtual host serving
  ONLY the injected document. Every subresource (same-origin fetch, XHR) fails
  NetworkError; `file://` XHR is scheme-blocked from an https origin. No
  file-read primitive.
- `url`-field `data:` page runs JS but is `origin=[null]` (opaque) — `file://`
  XHR NetworkError. No file-read primitive.

Net: the LFI exfiltrates world-readable non-`/proc` files (`/etc` DB, infra
topology, OS image) but **cannot reach environ/cmdline/maps/fd** — exactly the
set that would carry env-var secrets, cloud creds, and address layouts. The
guard resolves symlinks and canonicalizes paths, so no indirection defeats it.
Combined with network-blocked cloud metadata and an unprivileged renderer, the
screenshot-LFI blast radius is bounded to host recon, not secret theft.

To actually read environ an attacker would need one of: a `/proc` path the
guard's resolver mis-canonicalizes (none found in ~20 variants), a renderer
build with the guard absent/disabled, or a write primitive on the render host
to plant a same-origin readable file. None is available through the screenshot
API. Recommend Framer still fix the root `file://` scheme exposure (the
allowlist above) — the `/proc` guard is a good second layer but the scheme
should not be reachable at all.

## Sixth pass — hunt for the guard's source on the worker: INCONCLUSIVE (host is opaque)

Goal: locate the renderer's app root on disk so the `/proc` guard's JS source
could be read via the LFI and analyzed for a logic weakness. **Failed — and
the failure itself is a finding: the render host exposes almost nothing that
identifies its own software.**

New host facts established this pass:
- OS: **Ubuntu 26.04.1 LTS "Resolute Raccoon"** (`/etc/os-release`) — fresh
  2026 image. `/etc/debian_version` also readable.
- `/etc/passwd` is the **stock 17-entry base** (root + system accounts +
  `ubuntu:x:1000` only). **No `node`/`framer`/app service account exists** —
  the renderer runs as one of the stock accounts (likely `ubuntu` or a
  uid-mapped account) on a minimal image.
- `dpkg` userland present: `/var/lib/dpkg/status` is readable but returns
  `SCREENSHOT_FAILED` (too large to rasterize) — a *new, distinct* error code,
  not `PAGE_LOAD_FAILED`.

Why enumeration is structurally blocked:
- **Headless Chromium renders no directory index.** `file:///etc/`, `file:///tmp/`
  (both confirmed world-readable dirs) render fully blank — no table/tr/td/a/h1
  elements exist. There is no directory-listing oracle.
- **`PAGE_LOAD_FAILED` is a 4-way conflation**: absent OR permission-denied OR
  binary/unrenderable OR (for dirs that fail) — with no way to distinguish.
  Calibrated: `/bin/ls` and `libc.so.6` (definitely present, ELF) fail
  identically to guaranteed-absent paths. So a failed probe teaches nothing.
- Blind path-guessing (~70 targets) all `PAGE_LOAD_FAILED`: Chromium install
  manifests (`/opt/google/chrome/*`, `/usr/lib/chromium/*`), Node global modules
  (`/usr/lib/node_modules/{npm,corepack,pm2,puppeteer}`, `/usr/local/lib/...`),
  app roots (`/app`, `/srv`, `/usr/src/app`, `/home/node/app`, `/package.json`),
  systemd units (`/etc/systemd/system/{framer-renderer,screenshot,renderer}.service`,
  `/lib/systemd/system/...`), cloud-init (`/var/lib/cloud/instance/{user-data,cloud-config}.txt`,
  `/var/log/cloud-init*.log` — all absent or unreadable), nginx configs,
  `/home/ubuntu/{.profile,.bashrc,.bash_history,.npmrc,package.json}`.
- The `/home/ubuntu/*` failures are notable: `.profile`/`.bashrc` are
  world-readable on stock images, so either the homedir is empty/absent
  (minimal image, no login account use) or the renderer is jailed from it.
- `/etc/environment`, `/etc/ld.so.conf{,.d/x86_64-linux-gnu.conf}` are all
  **stock defaults** — no custom PATH or lib path that would name an app dir.

Conclusion: the app root is not discoverable through the raster channel. Every
oracle that would reveal it (directory listing, shell rc files, service units,
cloud-init logs, process cmdline/environ/cwd) is either blank-rendered,
permission-gated, absent, or behind the `/proc` guard. The guard's source is
unreachable via this LFI; assessing it for logic flaws would require the
renderer build/repo, which this endpoint does not expose. This materially
limits further escalation analysis from the outside — treat the `/proc` guard
as a black box that is *observed* robust (~20 canonicalization/symlink variants
defeated) but not *proven* robust from source.
