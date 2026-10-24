# Fresh-login rate admission prerequisite

The selected identity path is a fresh verified sign-in. Source inspection on
`0b4cdde` identifies a separate prerequisite for an initially empty native host.
No live login, SDK call or account inspection was performed for this finding.

`camoufox_host.py` records verified user/session identity after successful
login completion. A later successful SDK observation and joined browser close
can retain its observation proof. This does not by itself satisfy native-open.

`NativeDevices.open` in `scripts/native_device.py` requires either matching
`_cooldown_restoration.generation` or matching `native_enrollment_generation`.
The former is established by a real startup transfer. The latter is established
by enrollment, which refuses an account already present in host lifecycles or
accounts and refuses an existing profile directory. A normal fresh browser
login creates that history and profile before a later native-open request.

Consequently, selecting fresh login does not make an empty package sufficient
for the first native inference. The launch must either bind a valid existing
rate-state transfer, or qualify an explicit same-host admission proof for the
intact ledger after joined browser close. Do not invent a restoration receipt,
reset rate history, enroll over existing history or treat cookies as proof.

No source correction is applied by this report. Next source work is a bounded
producer/admission contract for this exact condition, with the existing session,
observation, generation, cleanup and rate checks preserved. This prerequisite
is separate from the already-authorized normal-service rollout.
