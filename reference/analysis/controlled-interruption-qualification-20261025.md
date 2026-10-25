# Controlled interruption qualification, 25 October 2026

The unlaunched R5 controller implements the user's permission to interrupt
active requests during the approved f70458d3 repair rollout. It temporarily
restricts port 8787, preserves the listener and disconnects only connected
sockets owned by the bound main process. The restriction expires without
refresh and is removed when its controller ends. Genuine rate-state capture,
internal cleanup and rollback requirements remain.

Eight isolated kernel checks, ten retained controller cases, one four-scenario
flow case and seven new receipt cases support the controller. The 28 socket,
disconnect, policy and control-address cases are accepted. Their initial run
found a PID-reuse race: identity was checked before pidfd acquisition but only
rechecked after shutdown. Rechecking immediately after acquisition, inside
owned cleanup, passes all 12 affected disconnect cases. The other 16 passes
are retained. Lease refreshes create separate receipts, preserving prior files.

The 20 application repair cases and 55 authority cases remain accepted without
rerun. R5 is bound but not deployed. Live interruption, a completed subsequent
stream and restored rate-state evidence remain required. No native/V3
acceptance or resource-outage prevention is claimed.
