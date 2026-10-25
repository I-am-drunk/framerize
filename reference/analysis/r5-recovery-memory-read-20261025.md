# Recovery-only finalization: memory-read permission failure

The existing recovery controller ran once after user-provided local sudo
authentication. It performed no binary switch and sent no inference request.
The temporary ingress restriction interrupted one bound client socket.
The controller reached a quiet state and obtained the restoration proof.

Finalization then failed with `PermissionError` while opening the bound
sidecar's `/proc/PID/mem` read-only. This occurs before the main/sidecar hold,
value comparison, transfer clear and restart-fence removal. None of those
completion steps is claimed. The authenticated ingress helper exited normally
and recorded removal of its temporary restriction.

Controller terminal 2536 completed with exit 1; authentication terminal 29598
completed with exit 0. All frozen source inputs remained unchanged. The known
controller/helper processes and transient unit were absent after collection,
and the shared execution lock was free. The prior binary remained active and
running under the same service invocation. Its owned restart fence remains.

A finite source correction is assigned to provide only the needed read-only
descriptor from the authenticated helper to the exact bound controller. State
comparison, generation checks and clear ordering must remain intact. No
ptrace-policy relaxation, broad capability grant or second switch is authorized
by this correction. The failed recovery packet remains consumed and preserved.

Recovery completion, repair deployment acceptance and browserless native
acceptance remain separate, unfinished runtime results.
