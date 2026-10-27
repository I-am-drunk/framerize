# Build runners

The build workflow on `browserless` runs in the main `I-am-drunk/framerize`
repository on Iris. It no longer depends on the `diddletime` CI mirror.
Jobs require the labels `self-hosted`, `Linux`, `X64`, `iris`, and `framerize`.

Iris runs one application job at a time in a fresh container. Each job has
1.5 CPU cores, 3 GiB of memory and a 4 GiB combined memory/swap limit.
Cargo uses one build process. The 90-minute timeout allows a cold build on
the shared server. The container has no host directories or Docker socket.
Runner registration uses the owner's existing private GitHub CLI login;
the container receives only a single-use runner configuration.

The runner service is `fleet-runner@framerize.service` in the server user's
systemd instance. It starts on boot and replaces the container after each job.
Its maintained deployment files live in Tjae's Fleet under `ci/runners/`.

GitHub currently charges no compute minutes for self-hosted runners.
The existing GitHub cache and uploaded build artifacts still use GitHub
storage allowances. Their storage is separate from runner compute.

The legacy `jwt-relay.yml` workflow is unchanged. It explicitly depends on
GitHub-hosted runner IP addresses, so changing its runner to Iris would
change its behavior. It is not part of the application build migration.
The default `main` branch and the running application service are unchanged.

To inspect a failed build, open its Actions run and check which step failed.
To inspect the runner itself on Iris, run:

```sh
systemctl --user status fleet-runner@framerize.service
journalctl --user -u fleet-runner@framerize.service -n 50
```
