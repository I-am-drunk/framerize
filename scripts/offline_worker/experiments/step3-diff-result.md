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
