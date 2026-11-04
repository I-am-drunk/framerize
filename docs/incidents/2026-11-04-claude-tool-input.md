# Claude tool-input budget repair

The reported error is generated locally by Messages continuation handling.
Ordinary checkpoints stop at 1,200 tokens to reduce upstream request latency.
Tool recovery also applied that ceiling, then refused when its next allowance
could not grow. This could reject a tool with substantial client budget left.
Recovery from a signed prefix also discarded the computed larger allowance.

The repair separates tool retries from ordinary checkpoint pacing. An
acknowledged, unpublished client tool can double its allowance for each of
the existing two retries, bounded by the remaining client output budget.
For example, 600 and 1,200 token truncations can receive a 2,400 token attempt.
The computed allowance also applies after preserving a signed or text prefix.
All acknowledged usage, including discarded input, counts against the total.
Ordinary sizing resumes when the reply no longer contains an incomplete tool.

The change preserves caller thinking settings, history format and contents,
model/account ownership, partial-input withholding, server-tool restrictions,
request count and active-time limits, and unknown-completion replay refusal.
Explicitly opted-in metered fallback retains its previous trigger.
No live request, installed setting, database or recovery guard is changed.

A finite retry budget cannot complete arbitrarily large tool calls. Exhausted
client/retry budgets still fail safely. Larger buffered allowances can still
encounter the upstream deadline; synthetic fixtures cannot qualify delivery.

Eight new synthetic chain regressions cover larger complete input, preserved
signed prefixes, remaining-budget clamping, the finite retry bound without
a client total, explicit thinking, unknown completion, restored prose sizing
and exhausted budgets. The larger-input regression reproduced the exact
reported error before the fix. All 177 focused Messages tests pass afterward.
The existing tests cover metered fallback, malformed input, cancellation,
server tools and checkpoint-history restoration. No live inference was run.

Alpha.31 includes this repair above the alpha.30 fixed host-memory floor.
Alpha.29 remains the running deployment until exact-package qualification
and the existing guarded graceful activation succeed. Prior alpha.29 evidence
does not qualify this changed binary. The alpha.30 executor pins different
hashes and must not be used unchanged for alpha.31.
