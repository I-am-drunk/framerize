# T3 Code attribution

Framerize Remote reuses source adapted from T3 Code
(https://github.com/pingdotgg/t3code), MIT License, Copyright (c) 2026
T3 Tools Inc., at commit fbe5df2d4b630d13adc8fe2d38cab354e6d66d67.

## Adapted files

- `src/hostPolicy.ts` adapts the loopback classification from
  `packages/shared/src/hostClassification.ts`: `normalizeHostname` and the
  127.0.0.0/8, localhost, and ::1 loopback tests. The Effect dependency and the
  tailnet, private-network, and favicon checks were dropped.
- `src/cliArgs.ts` adapts the declared-option argv parsing approach from
  `packages/shared/src/cliArgs.ts`: accept only declared options, reject
  duplicates, and never place raw argument values in error messages.

The upstream MIT License text is retained alongside the Framerize LICENSE. No
T3 Connect OAuth, relay, secret-store, or service-backend code is included.

`src/tailscale.ts` adapts `packages/tailscale/src/tailscale.ts` from the same
pinned revision. It retains direct platform-specific executable selection,
`status --json`, the 1,500 ms deadline, fixed stderr labels and tailnet IPv4
classification. Node built-ins replace Effect. Combined output is bounded,
the subprocess environment is restricted and reported identities are omitted.
No login, Serve, Funnel or configuration command is included.
