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

The installer only wraps the existing host command/account/status boundaries.
Private device values travel in the existing command reply. No credential or
device value is included in public errors or diagnostics.

Status: source only, untested and not installed. Fixtures use temporary state
and synthetic clocks. Root still owns Rust integration, protocol validation,
bounded execution, independent review and any deployment.
