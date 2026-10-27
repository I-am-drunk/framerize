# Build runners

This describes the proposed runner topology. Framerize runner registration and
end-to-end execution remain unverified. Keep `FRAMERIZE_PRIVATE_CI_READY` unset
until platform-side admission and the `framerize-ci` environment are verified.
PR-editable workflow predicates cannot enforce that trust boundary. The aggregate
`Check` job and alpha draft publisher use hosted runners, not this private runner.

The build workflow on `browserless` runs in the main `I-am-drunk/framerize`
repository on the iMac. It no longer depends on the `diddletime` CI mirror.
Jobs require the labels `self-hosted`, `Linux`, `X64`, `imac`, and `framerize`.

The iMac runs one application job at a time in a fresh Linux container.
Its Lima VM has four CPUs, 6 GiB of memory and a 40 GiB disk. Each container
has three CPUs, 5 GiB of memory and a 6 GiB combined memory/swap limit.
QEMU emulates a complete x86_64 Linux machine and preserves Linux artifacts.
The tests require Linux procfs and the x86_64 worker syscall sandbox.
Rosetta cannot apply that sandbox and changes missing-executable behavior,
so it cannot run this test suite correctly.
Cargo uses two build processes, with a 180-minute job timeout for cold builds
under full system emulation. GitHub's Rust cache reduces later compile work.
The VM shares no user folders. Containers have no mounts or Docker socket.
Runner registration uses the owner's existing private GitHub CLI login;
the container receives only a single-use runner configuration.

The runner service is `fleet-runner@framerize.service` on Iris. It registers
jobs and controls Docker in the iMac VM through a restricted SSH key over
Tailscale. Build and test processes run on the iMac. The connection starts
the VM when needed and replaces its container after each job.
Its maintained deployment files live in Tjae's Fleet under `ci/runners/`.

GitHub currently charges no compute minutes for self-hosted runners.
The existing GitHub cache and uploaded build artifacts still use GitHub
storage allowances. Their storage is separate from runner compute.

The legacy `jwt-relay.yml` workflow is unchanged. It explicitly depends on
GitHub-hosted runner IP addresses, so changing its runner to the iMac would
change its behavior. It is not part of the application build migration.
The default `main` branch and the running application service are unchanged.

To inspect a failed build, open its Actions run and check which step failed.
To inspect the supervisor on Iris, run:

```sh
systemctl --user status fleet-runner@framerize.service
journalctl --user -u fleet-runner@framerize.service -n 50
```

The job's runner name is `imac-framerize`. On the iMac, `limactl list`
shows the `framerize-ci-x86` VM. Both Iris and the iMac must be reachable.
FileVault can require unlocking the iMac after a reboot. Fleet's native
macOS runner remains separate and keeps the Mac awake while enabled.
