# Browser deployment outage on 2026-10-27

The deployment controller stopped the working browser service at 19:05 UTC.
Its direct script entry then raised `NameError: name 'switch' is not defined`.
The function appeared below the script's `__main__` block. Import-based checks
loaded that function before calling the controller; direct execution did not.
Both in-process recovery and the systemd recovery command called the same
unavailable function and failed. The service stayed stopped.

The journal records the first later start at 20:38 UTC, followed by repeated
recovery restarts through 21:19 UTC. These timestamps do not establish that
the service was usable between restarts. The user also had to restore accounts.
The deployment journal does not establish which recovery action changed them.

The failed candidate never replaced the browser executable. The recovered
service uses browser commit `7ccd0591c52b1f9ac7d153f1bac92f454a97cad4`.
The controller and its consumed plan must not be replayed. Stale pool settings
and history backups must not overwrite the user's recovered state.

## What the checks missed

Package startup and simulated low-memory checks exercised the application,
not the deployment command's direct entry. Six controller checks did not
execute the installed command and its recovery command as fresh processes.
Those results remain valid only for the paths they actually exercised.

## Required release separation

1. Submit source changes through reviewed PRs. Passing fixtures alone cannot
   promote code to a serving branch or replace an installed binary.
2. Build an identifiable package once, with its version, commit and checksums.
   Retain test results for that artifact and publish alpha releases explicitly.
3. Test the actual installer and rollback entry points against a disposable
   service, including failure immediately after stopping it.
4. Treat production activation as a separate operation. Preserve accepted
   streams, current account state and new history writes. Refuse activation
   if its independent recovery path has not been demonstrated.

No replacement production controller is qualified by this document. The
current review/release work does not restart the service or alter its accounts.
