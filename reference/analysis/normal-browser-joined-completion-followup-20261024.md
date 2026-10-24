# Normal-browser joined completion follow-up

Status: source assignment only, based on qualified `95f29867`. No new test,
build, executable launch or service change has run for this follow-up.

The prepared normal-browser rollout remains preserved. Its acceptance plan
requires an interrupted producer to finish, its task to be joined and its
registration to be released before the subsequent normal-port stream.

The existing `spawn_observed` sends its cleanup verdict inside the producer,
before the wrapping task completes its child registration and returns. When
the HTTP body disappears, its bare task handle is dropped and the completion
observer waits for that verdict. That is not observation of the original task
join. This is an ordering correction as well as an evidence requirement.

The authorized source change shares one original task join between the body
and its completion observer. It must preserve the original result and
`JoinError`, including non-Clone result values, accumulated cleanup uncertainty,
disconnect behavior and the unmanaged-host barrier. It adds no producer task,
retry or core gate API. Body disposal must retain the join observer.

Server-generated opaque root and child IDs correlate completion events. Only
the root ID is returned in a response header, and only when ownership is
active. Join outcome and successful registration release are distinct events.
Release is reported only after `OperationOwner::complete` succeeds; unresolved
paths retain the fence. Events run outside owner locks and carry no request
credentials, bodies or caller-supplied identities.

Server owns the ownership module, two existing streaming chain-event fields
and test-module wiring. A separate fixture owner covers held producers after
disconnect, clean EOF, unknown cleanup, original join errors, once-only release
and concurrent identity/privacy separation. Changed-helper controls accompany
the focused selection. The source and fixture handoff remains pending.

This work is independent of ordinary V2. A qualified follow-up will supersede
the preserved rollout candidate for joined acceptance. The protected-service
restriction and the separate live rollout decision remain in force.
