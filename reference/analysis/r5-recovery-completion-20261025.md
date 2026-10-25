# Rollback recovery completed, 25 October 2026

Recovery r6 completed against the unchanged previous binary. All eleven
account lifecycle records and their enforced limits passed verification.
Three primary selections had changed from null to resident devices belonging
to those same accounts. The new predicate accepted only that supported case.
Initial restoration, inventory and generation checks, active cooldowns,
request multiplicity, mint spacing and daily reserves remain enforced.

Thirteen new focused tests passed without skips, including rejection of
cross-account device substitution and each category of limit loss after an
otherwise compatible selection. Twenty-one unchanged memory, lease,
provenance and summary cases were retained. The frozen failed attempts and
their original comparison remain preserved.

The live verifier compared state before clearing the pending transfer.
It then removed the owned restart fence and rollback drop-in. Both service
processes resumed, all helpers were collected, and the temporary ingress
restriction was removed. The service remains on the same previous binary
and generation with `Restart=always`. No further binary switch or inference
request occurred. Two bound client sockets were interrupted during recovery.

Controller terminal 68103 exited 0; authentication terminal 10131 exited 0.
The collection confirms unchanged source pins, no remaining helper, unit or
private socket, and a free execution lock. Collection SHA256:
`bf6563ce14e1f296ba222574fa891f02b96c63b0ebc1d7a45fd22e973c42df52`.
The source-bound plan was
`03f822314139c75eb24d6a2794ce0291211aba071086e04e8b49d812f86d1c58`.

This completes rollback recovery. It does not deploy the repaired candidate
or establish its stream acceptance. The next repair step is a fresh rollout
using the qualified 240-second startup budget, followed by two stream checks.
