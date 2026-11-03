---
name: test-framerize
description: Verify Framerize server, Remote and installed-package behavior with isolated synthetic state. Use for integration checks and CLI workflow verification, not routine documentation edits or unrequested live inference.
---

# Test Framerize

Start with the changed behavior and the result it must demonstrate. Read the
current package scripts and the selected fixture before running it. Build only
the required outputs; a test using stale dist files does not check new source.

## Pick the relevant entry point

- Core HTTP behavior: `framerize-core/test/`. Its package test script compiles
  TypeScript into `.test-dist`; select the affected compiled test when possible.
- Provider, account and CLI behavior: `framerize-server/test/`.
- Local forwarding and connection UX: `framerize-remote/test/`.
- Installed behavior: root `test/installed-server-flow.test.ts` and
  `test/installed-remote-flow.test.ts`, compiled through `tsconfig.tools.json`.
  Inspect their build and archive prerequisites rather than assuming a source
  test or an existing archive is equivalent to the current installed package.

## Isolate state and network

Use private temporary HOME/state directories, synthetic credential references
and loopback fixtures. Do not read live sessions, import accounts, start against
the user's data, or expose a listener beyond loopback for an ordinary test.
Keep captured production payloads out of fixtures unless explicitly approved
and sanitized. Meaningful synthetic failures are better than an empty setup.

Reuse a healthy fixture owned by this task when appropriate. Otherwise retain
the spawned process handle and actual assigned port. Wait for a specific ready
event or bounded readiness check, not a fixed sleep. Never kill by process name.

## Exercise the whole changed path

For a forwarding change, send the request through the real local listener to
the synthetic upstream. Check the response and what the upstream received.
Include the applicable failure or reverse operation: invalid credentials,
unavailable upstream, cancellation, disconnect, or shutdown. Verify cleanup
after the operation settles, not just a successful response status.

For an installed CLI change, run the packed package outside the checkout.
Check first use, a second invocation and the relevant failure message. Keep
credential values out of output. Do not claim registry installation from a
local tarball, or native/platform coverage from one JavaScript process.

Report the source revision, exact commands, environment, observed behavior
and untested limits. Leave an inspection server running only when requested;
otherwise close owned processes and remove owned temporary state.

Live inference is separate. It needs the agreed account, endpoint, model,
streaming mode and request count. Never silently switch models, accounts,
transport or billing assumptions to obtain a passing result.

See [attribution](../../ATTRIBUTION.md) for the T3 testing pattern adapted here.
