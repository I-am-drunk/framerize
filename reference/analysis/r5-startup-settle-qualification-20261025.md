# R5 startup settling correction

The collected R5 attempt took 124.114 seconds to finish six sequential browser
warmups. Its 90-second post-start check refused before that work had finished.
The returned correction changes only the two post-start quiet budgets in
`controller.py` and `rollback.py` from 90 to 240 seconds. All other bytes match
the frozen R5 controller. The 45-second guard acknowledgement and 120-second
rollback startup/receipt deadline remain unchanged.

Seven selected inert Python tests passed, covering 20 executed subcases with
zero skips. The tests extract the bound caller statements and readiness
functions, replace external dependencies with inert fixtures and use a virtual
clock. The observed schedule passes at the first settled sample, 125 seconds;
the original budget refuses it. Busy or stale readiness still refuses at 240,
and generation, guard, ingress and coordination failures still abort.

Qualification used an isolated network/PID/filesystem view, a 128 MiB memory
cap, zero swap, 32 tasks, one CPU and the shared execution lock. All bound
inputs remained unchanged. The launcher, child and transient unit were absent
after collection, and the execution lock was free. This is fixture evidence,
not a second rollout or live stream-acceptance result.

Bindings:

- Corrected controller: `a9cb3e0ee954b48cd801616746fa0862cd831a3de36246406a449b0ed51472f1`.
- Corrected rollback: `cadf1b92b0dafb89d3076f2aa4710d3541dc95d00d4bcf9f3d59ed14c3b2e6e5`.
- Fixture: `d4b2a3c731abc087d7d47085c5cde1cac5f9be059ceaacfd840ed55a4f981bfc`.
- Result: `6e327c0f66c82c0d3d5554cbe3a7a724b7d83c1019fd1d85a0b6b1e7739ece76`.

R5 remains consumed. Recovery-only completion retains its separate existing
authentication dependency and permits no service switch or inference request.
The qualified correction has not been integrated into a new launch packet.
