# Screenshot url-field — file:// scheme Local File Read (CONFIRMED, 2026-10-09)

Route: `POST /web/agents/screenshot` with `{url, theme, viewport, selectors[]}`.
The zod schema is `url: z.string()` — NO scheme validation. The SSRF guard is
DNS-resolution-based and only engages for http(s) hosts; `file:` has no hostname
to resolve and falls straight through to the headless browser's loader.

## Confirmed primitives (all from a FREE account, cookie-only auth, no Castle)
| url | result |
|---|---|
| `file:///etc/passwd` | rendered full contents (root, ubuntu, etc.) |
| `file:///etc/hosts` | rendered — **internal topology: EC2 10.1.12.187, ip-10-1-12-187.ec2.internal** |
| `file:///etc/hostname` | rendered |
| `file:///etc/os-release` | rendered |
| `file:///proc/self/environ` | content READ (renders as 8px line, null-bytes collapse layout) |
| `file:///proc/1/environ` | exists (SELECTOR_NOT_VISIBLE = exists-but-not-visible) |
| `file:///proc/self/cwd/` | renders (empty dir index) |

## Error-code oracle (file existence primitive)
- `PAGE_LOAD_FAILED` = file does not exist
- `SELECTOR_NOT_VISIBLE` = file EXISTS but content not renderable at that size
This gives a clean boolean file-existence oracle over the whole filesystem.

## Outbound channel (exfiltration vector) — CONFIRMED
A `data:` URL page in the url-field executes JS AND makes outbound subresource
requests (`<img src>`). Verified via canary (webhook.site) — request arrived
from Framer's render infra (Chrome/149 headless, AWS us-east-1 52.21.124.57).
So arbitrary file bytes can be exfiltrated if they can be read into JS.

## Open sub-problem
`fetch("file://...")` is blocked (TypeError). The read channel that works is
url-field NAVIGATION (renders text/plain but no JS on text/plain pages).
Next: XHR / iframe+DOM read / `<object>` of file:// from a data: page to get
content into JS, then exfil via <img> query string.

## Exfiltration-channel deep analysis (2026-10-09, continued)

The url-field navigation to file:// is the ONLY working read channel. All
JS-mediated reads of file:// from a data:/web origin are blocked by Chromium's
web-content file-access sandbox (sync XHR, async XHR, fetch, <iframe>,
<object>, window.open all deny — verified via canary).

Newline-separated files read CLEAN through the raster:
- /etc/passwd, /etc/hosts, /etc/hostname, /etc/os-release, /etc/resolv.conf
  (resolv: nameserver 10.1.0.2, search ec2.internal — internal AWS topology)

Null-separated proc files (environ, cmdline) read but DON'T raster legibly:
- Chromium lays out the single null-joined line; the selector box collapses to
  ~8px height (SELECTOR_NOT_VISIBLE at any viewport that doesn't match).
- view-source: wraps but renders the nulls as invisible -> blank.
- iframe/object/XHR bridges to file:// are sandbox-blocked.
- This is a RASTER/layout constraint, NOT a read constraint — the file content
  IS returned to the headless browser (the hairline is the laid-out line).

## The remaining solve (root-class: environ = AWS/DB creds on EC2)

Mechanism PROVEN 2026-10-09 (second pass):
- `data:text/plain` page with environ-structured NUL-separated bytes renders
  PERFECTLY — Latin-1 fallback, NULs visible as U+FFFD replacement glyphs,
  full-height raster (`PATH=/usr/local/bin HOME=/root AWS_SECRET=AKIA...`
  control decoded completely).
- `file://` navigation of the SAME bytes collapses: Chromium's file loader
  sniffs alternating NULs and applies **UTF-16 charset detection**, decoding
  byte pairs into CJK/unassigned codepoints with no font glyph → zero advance
  width → 8px-height blank strip (899-byte all-white JPEG, text genuinely
  ABSENT from raster, not tiny — upscaling recovers nothing).
- text-fragment `#:~:text=PATH` on the file:// URL does NOT match (the DOM
  text is the mangled UTF-16 decode, not the raw bytes).

Bridges tested and closed (second pass):
- url-field `data:` page runs JS but is `origin=null` (opaque) — sync XHR to
  file:// → `NetworkError: Failed to load` (`--allow-file-access-from-files`
  NOT set on the render browser).
- htmlBase64 page runs JS from `https://screenshot.framer.invalid` (real
  https origin via renderer-internal virtual host) — sync XHR to file://
  → `NetworkError` (mixed-content/scheme block); the virtual host serves
  ONLY the injected document, no path mapping to the filesystem.
- meta-refresh `data:`→`file://` redirect chains do not fire in headless
  (blank render, no navigation).
- `<img src=file://>` → onerror; `<object data=file://>` → blank;
  `<iframe src=file://>` → blank; FileSystem API → sandboxed virtual FS
  (`GETFILE_EX NotFoundError`); CSS `@font-face`/`@import` of file:// →
  blocked; chrome:// pages → PAGE_LOAD_FAILED; `javascript:`/`blob:`/
  `filesystem:` top-level → PAGE_LOAD_FAILED.
- `<meta charset>` IS honored in htmlBase64 (UTF-16 control decoded
  correctly) — but charset cannot be forced on a file:// navigation, and
  file:// content can never reach a page where meta applies.

The wall is exact: environ's bytes are readable ONLY through the file://
loader, which destroys them via UTF-16 detection. Every path that preserves
the bytes (data:/htmlBase64) cannot obtain them. The render binary is
server-side — flags/binary patches are not attacker-reachable from a free
account.
