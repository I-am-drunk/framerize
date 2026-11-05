# T3 Code attribution

Server setup reuses source from T3 Code
(https://github.com/pingdotgg/t3code), MIT License, Copyright (c) 2026
T3 Tools Inc., at commit 5a906583fed3fa41dc806f2a19bd9d095537ed3e.
The upstream license text is in `T3-LICENSE`.

## Adapted files

- `src/startup-access.ts` adapts `apps/server/src/startupAccess.ts`:
  `isWildcardHost`, `formatHostForUrl`, `resolveHeadlessConnectionString`,
  the virtual-interface filter, `buildPairingUrl` and `renderTerminalQrCode`.
  The pairing URL is `<origin>/pair`, with `#token=` only when Server has a token.
- `src/private-network.ts` adapts the private IPv4, IPv6 loopback and ULA
  ranges from `packages/shared/src/hostClassification.ts`, and the
  private-listener socket check from `startupAccess.ts`.
- `src/setup.ts` adapts `apps/server/src/cli/up.ts`: access choice, route
  selection, service reconciliation and the printed pairing result.
- `src/service-units.ts` adapts the systemd unit, launchd plist and manager
  selection from `apps/server/src/cloud/bootService.ts`.
- `src/service-control.ts` adapts activation, rollback, status, linger repair
  and `runStep` from `cli/up.ts` and `cloud/bootService.ts`. The pinned
  runtime, update handoff, downgrade checks and memory protection are omitted.
  Stopping the installed service before repair follows `bootService.ts`.
  The port probe, process-ID readiness match and combined rollback errors
  are Framerize additions.
  Signal cancellation, stopping owned POSIX process groups with a one-second
  grace period, verifying their termination with a bounded process-state
  check, waiting for command closure, reporting cleanup failures, and
  continuing rollback after cancellation are Framerize additions.
- `src/tailscale.ts` adapts `packages/tailscale/src/tailscale.ts` and
  `apps/server/src/cli/pair.ts`: status parsing, Serve route inspection,
  `serve --bg`, removal and rollback. Unlike upstream `pair`, Framerize
  `pair` never creates a route; only `up` does. Moving a saved route to a new
  port and restoring a prior route are Framerize additions.

## Vendored file

`src/qr-code.ts` is `packages/shared/src/qrCode.ts` from the same revision,
byte for byte, after one added provenance line. It is the Project Nayuki QR
Code generator, MIT License, Copyright (c) Project Nayuki. Its license is
embedded in the file and retained in `NAYUKI-LICENSE`.
