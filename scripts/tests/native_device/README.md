This source component connects native issuance to an existing host lifecycle.
It requires completed restoration, an absent browser, the saved primary device
and the unchanged canonical device file. Missing authority refuses admission.

Open retains the lifecycle's active command count. Mint admission shares the
same cooling/request dictionaries, enforces the configured mint spacing and
charges one possible request before returning. Charges survive cancellation.
An account lock remains held until the caller confirms actual worker cleanup.
Subsequent native owners recheck the rate window after acquiring that lock.

Close requires joined request cleanup. A classified risk refusal extends the
same device's cooldown; it cannot shorten existing cooling. Unknown cleanup
retains ownership and appears as unconfirmed through existing browser status.
Legacy account use is excluded while native owners are active. There is no
automatic lease expiry, alternate-device selection or identity generation.

The installer wraps the existing host command/account/status/closure boundaries.
Confirmed browser closure preserves its actual final primary device in the
same lifecycle. Incomplete or failed closure cannot authorize native use.
Private device values travel in the existing command reply. No credential or
device value is included in public errors or diagnostics.

The 23 component cases passed on 2026-10-22 using temporary state and synthetic
clocks. The Rust connection has now compiled; its focused tests and independent
review are separate. The component is installed in R9, but the closure handoff
correction is not deployed. Four focused checks now cover the two handoff cases
and the existing installer ownership cases; the other saved checks are reused.
The handoff fixture executes the actual host closure method with inert disposal.
The runtime owner still controls real-request validation and deployment.
