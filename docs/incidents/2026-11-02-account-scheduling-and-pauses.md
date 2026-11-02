# Kimi stalls and account scheduling, 2026-11-02

The alpha.13 gateway had twelve eligible saved accounts and six browser slots.
Seven accounts sent requests after its 06:02:01 UTC start; six handled almost
all traffic. The other five had saved sessions but no fresh authentication
evidence in this process generation. The service stayed untouched during diagnosis.

## Causes and repair

Automatic selection ranked current request load before browser warmth, but
warmth had no fairness limit. An idle resident always beat an equally healthy
unloaded account. At capacity, that preference could keep the same six browsers
resident indefinitely. The repair ages fresh selections. After eight passes
over the eligible pool, an overdue equal-load account outranks warmth. Existing
health, availability, account ownership and confirmed-close rules still apply.
No failed request is replayed under another account.

Starting every overdue browser together could temporarily remove all warm
capacity. While one newly selected cold account holds preparation, work or
cleanup ownership, further overdue replacements defer to equal-load warm
accounts. Ordinary load, health and safety checks still apply. The full
server suite passes 484 tests with this guard; exact source hashes match
the checked-out revision.

The global refusal guard escalates clustered security refusals from 150 seconds
to a maximum of 900 seconds. Its ten-minute quiet reset ran only when a second
account refused within 90 seconds. By then the previous refusal could never be
ten minutes old. The repair checks quiet time on the first new refusal, before
recording it. It does not cancel an existing pause or remove the guard.

A Kimi request entering the 11:58:49 UTC pause completed after 906 seconds.
An Astra request entering the same pause later also waited, for 127 seconds.
This was a shared admission wait, not evidence of model-specific generation
latency. The ordinary Kimi completion median in the inspected window was 5.6 seconds.

Most HTTP 403 refusals came from two accounts that also completed hundreds of
requests. Browser token minting succeeded throughout the inspected generation.
Framer did not disclose its private reason for those security refusals.

The model catalog describes supported routing, not instantaneous admission
health. Kimi remains supported and completed requests during this generation.
The existing protocol names Framer's upstream HTTP 403 in the error message
and emits a terminal failure if streaming has already started.
Suppressing that failure would misreport an unsuccessful request as complete.
The gateway does not replay that request under another account. Client-side
retention of partially received text remains the client's responsibility.

## Verification before packaging

Both new regressions failed on the original source. The scheduler used only
six of twelve accounts after 120 sequential fixture requests. A new refusal
cluster after quiet still produced a 900-second pause. Both pass after repair.
All 159 core, 95 protocol and 483 server tests pass. The server run initially
found four older fixtures assigning credits to a personal space. They now
model incomplete initial metadata followed by the full organization listing.
The credit-admission implementation is unchanged.

Twenty-two inert activation and rollback checks pass. The activation guard
requires all recorded inference and cleanup operations to finish, including
requests waiting without a resident-browser lease. It also waits for a logged
global refusal pause to expire. A post-activation security refusal is recorded
as deferred verification; it cannot count as successful live delivery.
