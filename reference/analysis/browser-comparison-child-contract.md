# Standalone comparison child

Source-only preparation, 2026-10-24. This is not live acceptance.

The `browser_comparison` example has no listener or service-management path.
It takes only `--control-fd N`, an inherited Unix socket whose peer must be its
parent with the same effective UID. It marks that descriptor close-on-exec.
Frames use a four-byte big-endian length and UTF-8 JSON. Input is limited to
96 KiB, output to 8 KiB, and each transfer to five seconds.

The parent first sends a version-1 plan without credentials. The child checks
the two distinct turns and native request contract, then replies `ready` with
the plan nonce. Only then does the parent send `start`, that nonce and the
session cookie over the private socket. The in-memory store permits one load
and rejects save/delete. Each token stays inside the existing account scope.

After both request paths and account/browser cleanup settle, the child sends
bounded `result` facts. The parent durably records them under its existing
exclusion and replies `recorded` with the nonce. This acknowledges the report;
it does not release exclusion or certify descendant teardown. The child then
sends `done`, including local pair success, and exits. Missing or wrong replies
fail the child. No credentials, prompt, response text or raw exception is sent
back, and the executable has no stdout report protocol.

The external controller remains responsible for fresh resource admission,
protected-service identity, isolated account/device selection, exclusion of
conflicting writers, real cooldown history, exact input pins and independent
process teardown. It must retain its hold through final cooldown publication
and restoration. A child result cannot replace those checks. The concrete
outer controller and its live sequence are still pending.
