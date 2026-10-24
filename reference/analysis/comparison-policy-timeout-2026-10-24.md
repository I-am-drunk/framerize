# Comparison policy timeout

R17, frozen at `ddb800d`, passed editor readiness and then failed at
`policy_rpc_timeout`. The policy remained unknown. This is before bearer
acquisition, minting and either inference leg. Both cleanup receipts completed;
the child exited naturally, its group disappeared, and the protected browser
was unchanged. Peak group memory was 1,280,491,520 bytes with no swap or limit
events. The controller parked the isolated owner with its full history.

The saved current web entry and canvas-history producer statically import the
policy and engine modules. The inspected entry, router and policy module have
no top-level await; the engine module is a synchronous leaf. Background
WebAssembly compilation is not awaited by the policy getter. This supports
cached imports on that saved path. It does not prove R17 loaded that exact graph
or establish where its timed-out evaluation stopped.

A network-disabled browser probe used the exact readiness and policy functions
with synthetic page state and tiny modules at the same URLs. The cached case
returned in 8.2 ms and the first-time-import case in 62.8 ms. Both returned an
allowed policy and had no pending RPC after normal context closure. Peak group
memory was 553,127,936 bytes; swap and limit events were zero. These two samples
validate the basic evaluation path in a small fixture, not real-page timing.

The policy evaluation now emits fixed, ordered progress markers. They retain
the existing two-second limit and policy checks. Host-side fixtures verify
prefix matching, bounded forwarding, privacy, listener removal, late-event
suppression and retained RPC ownership. All 140 Python and 61 JavaScript cases
pass. No SDK or account is used by those fixtures.

A second isolated browser probe validated actual console delivery. The normal
case returned in 17 ms with all seven markers. Deliberately stalled engine and
policy imports timed out at 2.009 and 2.006 seconds, leaving the corresponding
import marker as the last browser observation. Both pending evaluations stayed
owned until context closure, then settled. All three contexts closed normally;
peak group usage was 593,379,328 bytes, with zero swap or limit events. The
test unit and child group were absent afterward. No account, SDK or inference
was involved. Missing markers in a future live trial will still mean unknown.
