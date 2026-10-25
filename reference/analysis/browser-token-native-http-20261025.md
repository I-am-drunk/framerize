# Same-device browser token and native HTTP comparison

On 25 October, the isolated resident comparison returned complete, nonempty
HTTP 200 responses for both its browser baseline and native HTTP leg. The
native leg used a fresh browser-generated token. Its report has no failure,
`native_output_complete=true`, `cleanup_confirmed=true` and `host_fenced=true`.

The actual running command used binary `fe48a036`, generated host `a530ecb8`
and no resident V2 worker configuration. The bound compiled source selects
`rpc.mint_native()` when that configuration is absent. Account, user, team,
project and proxy matched; the resident protocol retains the same device and
page across the paired calls. A preceding normal browser request established
the required successful-browser provenance in that generation.

The earlier production browser baseline used a different account and is not
part of this pair. The R16 native-worker refusal is retained as separate
evidence, with a different startup configuration and generation.

Genuine R16 rate history was captured and restored before this run. Only the
launcher's additional request-age wait was removed. Host and account admission
remained active. After the pair, updated history was captured and the isolated
service retired. All 12 captured process identities were absent. The original
service on port 8787 retained its process and invocation; no rollout occurred.

Evidence is under `parallel-browserless-r1/resident-browser-token-r18` and its
`-retirement` sibling in root's investigation. The result SHA256 is
`4353362f1272f7a958a40cc4c2781bf2f467c8b347e185a77083e7bcf730570a`.
Startup-only socket-path and stale-output-name failures are preserved. The
unspent control was continued without repeating the successful normal call.

This demonstrates native HTTP compatibility with a browser token in this
configuration. It does not establish browserless token acceptance, ordinary
V2 package acceptance, a particular cause of the native-worker refusal, or
memory savings. The next experiment must isolate native token generation.
