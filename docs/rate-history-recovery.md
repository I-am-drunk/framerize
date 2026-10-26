# Rate history startup and recovery

Browser and native runtimes share the configured reserve limit and mint gap.
Changing the reserve limit preserves consumed reserves, device identities,
request windows and cooldowns. A reboot rebases restrictions onto the new
monotonic clock and records the new boot identity after recovery.

An interrupted, empty database initialization is retryable. An unversioned
database is adopted only when it has a recognized schema. Unknown schemas
and malformed account records are preserved and reported, never erased.

When browser startup finds saved profiles but missing history, automatic
recovery requires an exclusive systemd service owner and the daemon's actual
session-directory lock. Saved device inventories must be valid. Recovery
holds every device, including the ambient identity, for 600 seconds and
blocks unknown reserve usage for 24 hours. It does not generate replacement
identities, remove profiles, or treat missing history as unused capacity.

GET /api/health reports HTTP liveness and admission state. GET /api/ready also
validates local account history through the host, without browser or upstream
requests. Recovery verifies /api/ready and its process identity before
recording a compatible browser return as restored.
