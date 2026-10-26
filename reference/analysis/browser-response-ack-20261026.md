# Acknowledged browser response delivery

Implemented on base `36819fa5797c1509b72732c2bd85cadbc1059838` on
`fix/browser-response-bounds-20261026`. This is an undeployed experiment.
The [result data](browser-response-ack-20261026.json) records source identities
and the local browser result. The prior measured queue remains unchanged.

`queue_ack_poll.js` assigns each chunk a sequence number. The receiver starts
with acknowledgement `-1` and acknowledges a chunk only after receiving and
validating it. Until then, the browser replays the same immutable frame.
Acknowledging a chunk may also return the next chunk. Losing that reply and
retrying the acknowledgement replays the next chunk without releasing twice.

An outstanding chunk counts against the existing 131,072 UTF-16-unit queue
capacity. A full queue keeps its producer waiting until acknowledgement.
The byte limit remains 8 MiB. The receiver still builds one final host body;
this does not stream application output or bound browser network buffers.

A final chunk remains available until acknowledged. After its receipt, the
receiver confirms removal with a final acknowledgement. If that reply was
lost, a missing job is accepted only after the receiver already validated the
terminal frame. A missing job before that point fails delivery.

The real-browser fixture passed on browser `152.0.4-beta.31`. One control
and five injected RPC-loss cases each returned the exact 8 MiB body and
sent exactly one HTTP request. Faults occurred before poll execution, after
the first chunk, after acknowledgement returned another chunk, after the
final chunk and after final cleanup. Each injected fault caused one retry.
These are deliberate loss simulations around actual browser calls, not
measurements of naturally occurring driver failures.

The same run kept an unacknowledged full chunk charged to the queue limit
and joined cancellation while the producer waited. Oversized input retained
its body-failure phase and HTTP 200 head, then closed the peer connection.
Aborting a held response with an outstanding chunk produced a joined failure,
not success. All jobs were removed and all browser descendants exited.

The run used private network/PID namespaces, the existing execution lock,
a 2 GiB/no-swap cgroup and a 512-task limit. It finished in 10.961 seconds.
All 29 source pins matched afterward. There were no resource-limit events;
the unit was collected and inactive with no main process remaining.
This run establishes no new memory saving or latency comparison.

The standalone receiver uses the corrected experimental reader from the
previous milestone. Its 8 MiB byte bound and cancellation behavior are
unchanged. The original full-body and unacknowledged-queue measurements
remain evidence for those exact sources, not for this protocol.

The standalone fixture did not qualify the host lifecycle. The subsequent
[host integration](browser-response-host-20261026.md) preserves `Host._rpc`
ownership and the shared request deadline, with separate host and real-browser
qualification. Neither fixture establishes a new minter/browser connection
or a real Framer request.

Intake completed the independent `queue_ack_contract.cjs` reservation. All
twelve cases passed with fixture SHA256 `8e6561fb…`. The integration reuses
that result and the byte-identical reader and poll; the source remains frozen.
