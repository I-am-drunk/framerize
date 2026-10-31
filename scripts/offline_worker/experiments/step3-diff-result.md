# Step 3 result: what actually differs between a real browser and the realm

Method: captured the Castle probe table twice in the user's real Chrome
(`probe-capture.json`, then a full-fidelity re-run `probe-capture-full.json`
with no 32-byte cap, token minted live in the tab, tokenLen=5005). Diffed
the two genuine browser runs against each other, then against the realm.

## Deterministic (realm reproduces exactly)

- 40 static env fields (navigator, screen, window): 0 diffs vs live browser.
- The 300-call `mmmmmmmmmmlli` measureText cycle: byte-identical across two
  real-browser runs, and the realm now replays it 300/300 after the
  realm.mjs sync fix.
- The three 200x200 getImageData reads: stable image, first bytes identical
  across runs.
- Navigator tuple (UA, platform, vendor, webdriver=false): identical.

## Varies between two genuine real-browser runs (not a realm defect)

- The random probe string itself (0436M97H... vs 014T76Oj...).
- The 17 random 1x1 sample coordinates and their pixel values.
- The text-bar rect: width 194 vs 197, 0 vs 908 nonzero bytes.

## What this proves

If Framer rejected a token for having frozen values, a genuine second mint
from the same browser would also be rejected, because real runs do not agree
on the pixel/region values either. So value-mismatch on the random probes is
not the 403 cause. Castle must score consistency within one token plus
non-canvas signals, not cross-run equality.

## Defects found and fixed by this diff

- experiments/realm.mjs was stale (old by-text Map collapse), so the running
  service replayed all 300 short-string measures as one width. Synced to the
  fixed realm_globals.mjs; verified 300/300. Commit d0fc3b96.

## Remaining suspect for step 4

- Realm returns all-zero / wrong-pixel data for the 6 larger getImageData
  rects whose capture was truncated to 32 bytes (the text bar is genuinely
  nonzero past byte 32). Full capture now exists in probe-capture-full.json;
  the realm should be re-pointed to it so every replayed pixel is real.
- The deterministic signals (UA, screen, fonts, measureText cycle) are exact.
  That leaves the truncated-pixel replay and, failing that, TLS fingerprint
  as the named suspects for the single live request.

---

# Step 3 update (2026-10-31, second pass): realm vs real 765-slot collection

Method: dumped the realm's 765-slot collection (`/tmp/realm-collection-string.txt`)
and diffed it slot-by-slot against the real browser's 765-slot collection
(`/tmp/real-collection.json`, captured read-only in a blank tab, no network).

Baseline before this pass: 583/765 slots identical.

## Defects found and fixed in this pass

1. window.chrome key order was ["app","loadTimes","csi"]; real Chrome 155 is
   ["loadTimes","csi","app"]. Fixed by reordering makeChromeObj.
2. chrome.app had 3 keys; real Chrome has 7 (isInstalled, getDetails,
   getIsInstalled, installState, runningState, InstallState, RunningState).
   installState() returns 'not_installed', runningState() returns
   'cannot_run' (probed live). Fixed in makeChromeObj + surfaces JSON.
3. navigator was missing userAgentData/deviceMemory/mediaDevices entirely;
   real blank-tab Chrome has userAgentData===null as an own property and the
   other two absent. Added userAgentData:null. Slot 109 now records all three
   globals the SDK probes.
4. performance.memory was invented (used=10M, total=20M, limit=4294967296).
   Real headless Chrome reports used=total=10000000, limit=3760000000. Fixed.
5. performance.now.toString() leaked the arrow source "() => performance.now()"
   because the realm had no native-source spoofing. Patched
   Function.prototype.toString inside the realm context (realm_window.mjs) so
   every realm-defined function reports "function name() { [native code] }"
   through both fn.toString() and Function.prototype.toString.call(fn), with
   hasOwnProperty('toString')===false. The patch registers itself so
   Function.prototype.toString.toString() also reads native.

After fixes: 590/765 slots identical (up from 583).
## What the remaining 175 diffs are

- ~120 short base64 blobs (slots 8-599): per-session encrypted fields. XOR of
  matched real-vs-realm blobs shows a repeating structured delta, meaning both
  encode the same plaintext under different session keys. Not a defect; two
  genuine mints from the same browser also differ here.
- 84 slots differ between two back-to-back realm mints (session nonce, crypto
  salt, timing). Legitimate per-mint entropy.
- Slots 41/427 (bool) and 43/432 (large int): the boolean is the SDK's
  "interaction observed" flag. The real baseline collection was captured on a
  blank tab with NO interaction, so it reads false; the realm replays the real
  28s interaction trace, so it reads true. That is the intended behavioral
  content, not a defect. The adjacent large ints are performance.now() ages at
  mint time, which differ by wall clock.
- Slot 109 internal shape codes differ (0306 0302 2f 00 08 vs 0306 0307 02
  06 08): the three missing-global entries are present in both but encoded
  with a different "absent" shape tag. The data (the three global names) now
  matches; only the absence-encoding byte differs. Lowest priority.

## Remaining suspects for step 4 (unchanged)

Deterministic env signals (UA, screen, fonts, measureText cycle, plugins,
chrome.app, performance.memory, function sources) are now byte-exact. The
only untested layer is the TLS fingerprint, already matched via the uTLS
custom spec (commit 34801f83). Step 4's single authorized live request is
the remaining proof.

---

# Step 4 prep (2026-10-31): wire-request environment alignment

The passing browser path sends the AI fetch from inside the editor page on
https://framer.com (crates/framer-host/src/lib.rs ai_post). Code comments,
verified live 2026-10-02: "a Castle token is only accepted when the request
shares the minting environment, same TLS stack, same UA, same Origin. The
exact same envelope via reqwest gets risk-refused even with a perfect token."

Gap found in the browserless sender (scripts/tls_sender/prepare_live.mjs):
it set only content-type, x-requested-by, authorization, and a cookie header.
The realm mints with location https://framer.com/projects, but the request
carried no Origin/Referer/sec-fetch/sec-ch-ua/accept headers, so it read as
a different environment than the mint.

Fixes applied to prepare_live.mjs:
1. Added the browser-automatic headers a real Chrome fetch from the editor
   adds: accept, accept-language, origin: https://framer.com, referer:
   https://framer.com/, sec-ch-ua (+mobile, platform), sec-fetch-dest/mode/
   site, and the Chrome/155 user-agent the realm claims.
2. Body now matches the passing envelope (proxy.rs): model at top level, the
   client payload nested under request. Previously model sat only inside
   request.
3. No Host header in the map: h2 :authority derives from the URL host; a
   manual host header would not map to a pseudo-header.

Go build clean (scripts/tls_sender, go build ./...).

Environment now aligned: TLS fingerprint (custom uTLS, commit 34801f83),
UA (Chrome/155), Origin (https://framer.com), and the token all describe
one consistent Chrome/155-on-framer.com environment. Step 4's single
authorized live request is the remaining proof.

---

# Wire verification (2026-10-31): sender output matches real Chrome

Ran the Go sender against a local h2 echo over the exact uTLS fingerprint
(no external request). Captured the full request. Two non-browser signals
found and fixed:

1. Go's http2.Transport auto-added "Accept-Encoding: gzip". Real Chrome sends
   "gzip, deflate, br, zstd". Set DisableCompression and the exact value.
2. After the fix the wire carries: authority api.framer.com, POST
   /ai/v3/messages/, accept, accept-encoding (full value), accept-language,
   authorization, content-type, cookie, origin https://framer.com, referer
   https://framer.com/, sec-ch-ua trio, sec-fetch trio, the Chrome/155 UA,
   x-requested-by. Header set matches a real editor fetch.

The sender is now byte-consistent with the mint environment on TLS, UA,
Origin, and headers. Remaining proof is the single authorized live request.

---

# Editor residency fix (2026-10-31)

The passing browser path mints the Castle token on the project's editor page
(/projects/<id>), and Framer's full-chrome engine 403s when the mint happens
on /login or the bare /projects index (verified 2026-10-11 in framer-host
lib.rs: "login-page context scores as not a real editing session").

The realm had location hardcoded to https://framer.com/projects (no project).
Threaded the minter's validated target.url through buildRealmGlobals so the
mint presents the editor page location. prepare_live.mjs Referer now points
at https://framer.com/projects/<projectId> to match.

Mint still completes (765 slots) with location now on the editor page.
