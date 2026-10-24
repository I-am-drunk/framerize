# Normal-stream acceptance checker qualification

The isolated checker has ten passing synthetic cases and one passing local
socket case. These results qualify the checker, not the deployed service.
The live pair remains unspent; the authorized rollout still awaits the fresh
host-state transfer and completed controller.

The cases reject opening frames without generated text, missing identities,
incomplete or cross-request releases, uncertain or reordered completion,
completion before disconnect, incomplete EOF, extra attempts and private
fields in reports. The socket case reads the actual HTTP identity header and
observes connection shutdown after generated text.

Synthetic result SHA256:
`cf7494f21175d00afa33295d482c65101538977f8bc83d28b2b5e5197fdc0239`.
Socket result SHA256:
`d88e3b3fb89f15132779119df23f7cecb7fb35c0d957d02d4d75394ad98b0fc9`.
Both used 128 MiB, no swap, 32 tasks and isolated networking/private state.
Captured processes, units and groups are gone; full transient PID capture
was not established. Earlier application qualification remains unchanged.
