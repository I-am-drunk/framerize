# T3 Code attribution

Framerize Remote reuses source adapted from T3 Code
(https://github.com/pingdotgg/t3code), MIT License, Copyright (c) 2026
T3 Tools Inc., at commit fbe5df2d4b630d13adc8fe2d38cab354e6d66d67.

## Adapted files

- `src/hostPolicy.ts` adapts the loopback classification from
  `packages/shared/src/hostClassification.ts`: `normalizeHostname` and the
  127.0.0.0/8, localhost, and ::1 loopback tests. The Effect dependency and the
  tailnet, private-network, and favicon checks were dropped.
- `src/cliArgs.ts` adapts the declared-option argv parsing approach from
  `packages/shared/src/cliArgs.ts`: accept only declared options, reject
  duplicates, and never place raw argument values in error messages.

The upstream MIT License text is retained alongside the Framerize LICENSE. No
T3 Connect OAuth, relay or secret-store code is included.

`src/tailscale.ts` adapts `packages/tailscale/src/tailscale.ts` from the same
pinned revision. It retains direct platform-specific executable selection,
`status --json`, the 1,500 ms deadline, fixed stderr labels and tailnet IPv4
classification. Node built-ins replace Effect. Combined output is bounded,
the subprocess environment is restricted and reported identities are omitted.
No login, Serve, Funnel or configuration command is included.

## Revision 5a906583

These files adapt T3 Code at commit 5a906583fed3fa41dc806f2a19bd9d095537ed3e,
under the same MIT License.

- `src/serviceManager.ts`, `src/serviceUnits.ts` and `src/serviceControl.ts`
  adapt `apps/server/src/cloud/bootService.ts` (blob `1e053770`):
  `systemdManager`, `launchdManager`, `selectBootServiceManager`, the unit and
  plist renderers, and install, restart, uninstall, status and `runStep`.
  `serviceControl.ts` also adapts `readSystemdProblems`,
  `requireSystemdPrerequisites`, `formatBootServiceProblem` and
  `BootServicePrerequisiteError`: check the systemd user manager and lingering
  with 5-second probes, try `loginctl enable-linger --no-ask-password`, then
  check again. The pinned runtime, update handoff, downgrade checks, busctl
  unit-path lookup and memory protection are omitted. `src/serviceCli.ts`
  adapts the status text of `formatServiceStatus` from
  `apps/server/src/cli/service.ts`, without versions or memory protection.
- `test/service.test.mjs` adapts cases from `apps/server/src/cloud/bootService.test.ts`
  (`6809e764`), including the linger repair before install. Its cases for a
  failed repair, an unreachable user manager and macOS without `loginctl` are
  Framerize tests of the adapted logic.
- `src/cmdUp.ts` adapts `setUp`, `formatSetup` and `formatNextSteps` from
  `apps/server/src/cli/up.ts`. `src/frmCli.ts` also adapts its `connectCommand`
  alias by calling the existing setup flow instead of maintaining a second one.
  The setup behavior comes from
  `apps/server/src/cli/up.ts` (blob `5a0e8209`): save the connection, check
  readiness, reconcile the service, restart it only when its settings changed,
  and restore the previous profile on failure. LAN, Tailscale Serve and
  pairing-token issuance stay in Server. `test/frm-up.test.mjs` adapts the
  restart and rollback cases from `apps/server/src/cli/up.test.ts` (`10fb3b42`).
  Repairing lingering for an unchanged running service without a restart is
  a Framerize addition, with its own case in `test/frm-up.test.mjs`.
  The local port check, matching listener and service process IDs, cancellation
  through setup, and reporting both setup and rollback failures are Framerize
  additions. Rollback runs after cancellation and preserves the previous unit's
  enabled and running state. Their cases are in `test/frm-up.test.mjs`,
  `test/frm-up-arguments.test.mjs` and `test/frm-service-ready.test.mjs`.
  Waiting for a cancelled service command to close before rollback, and force
  stopping that command after a one-second grace period, are Framerize additions.
  `src/serviceCommand.ts` extends this to owned POSIX process groups and checks
  their exit with `/bin/ps`, following Framerize Server's command runner.
  Waiting for concurrent probes, reporting cleanup failures, and forwarding
  cancellation from `frm service` are also Framerize additions.
- `src/hostClassification.ts` copies `packages/shared/src/hostClassification.ts`
  (`29648b45`) through `isPrivateNetworkHost`. The favicon classification
  after it is omitted. `src/remoteUrl.ts` uses it to allow plain HTTP to
  private hosts.
- `src/tailscale.ts` also adapts `parseTailscaleStatus`,
  `normalizeMagicDnsName`, `isTailscaleIpv4Address` and
  `buildTailscaleHttpsBaseUrl` from `packages/tailscale/src/tailscale.ts`
  (`4eb20f11`), without Effect Schema. Remote still runs no Serve or Funnel command.
- `test/pairing-remote.test.mjs` adapts Tailscale cases from
  `packages/tailscale/src/tailscale.test.ts` (`96f6e772`) and host cases from
  `packages/shared/src/hostClassification.test.ts` (`4110c9f2`).
- `src/pairingLink.ts` reads the `<origin>/pair#token=` format produced by
  Server's adaptation of `buildPairingUrl` from `apps/server/src/startupAccess.ts`.
- `src/compatibility/maintenance.ts` adapts install-path classification and
  the updater ownership order from
  `packages/provider-core/src/server/maintenanceResolver.ts` (`364b4cee`),
  `apps/server/src/provider/Drivers/ClaudeMaintenance.ts` (`d9975762`) and
  `CodexDriver.ts` (`871ce9ed`).
- `src/compatibility/latest.ts` adapts the latest-version lookup order from
  `resolveClaudeMaintenance` and from `withConfiguredNpmVersionLookup`,
  `resolveHomebrewProviderMaintenance` and the Chocolatey lookup in
  `resolvePackageManagedProviderMaintenance` in `maintenanceResolver.ts`.
- `src/compatibility/versions.ts` adapts `decodeNpmViewVersion`,
  `parseHomebrewLatestVersion` and `deriveVersionAdvisory` from
  `maintenanceResolver.ts`, and `readUpdateChannel` from `ClaudeMaintenance.ts`.
  `conflicts.ts` takes the managed settings paths from `readUpdateChannel`.
- `src/compatibility/semver.ts` adapts `normalizeSemverVersion`, `parseSemver`
  and `compareSemverVersions` from `packages/shared/src/semver.ts` (`720ea68a`).
- `src/compatibility/process.ts` adapts `escapeWindowsShellArg` and the
  `.cmd`/`.bat` dispatch through `cmd.exe` in `resolveSpawnCommand` from
  `packages/shared/src/shell.ts` (`28b666ce`).
- `src/compatibility/platform.ts` adapts `resolveWindowsPathExtensions`,
  `resolveCommandCandidates` and `stripWrappingQuotes` from the same file.
- `test/client-setup-paths.test.mjs` adapts cases from
  `packages/provider-core/src/server/maintenanceResolver.test.ts` (`9447742c`).
- Installers, Claude settings writes, conflict checks and launch overrides
  are Framerize code. So is the version-probe cleanup in `process.ts`. At
  its deadline, size limit or setup cancellation, it closes stdout and stops the probe's process
  group on POSIX, or runs `taskkill /T /F` on Windows. Only a fake runner
  tests the Windows branch.
  Passing setup cancellation through version lookups and release-body reads
  is also Framerize code.
  Cancellation of running installers and updaters is Framerize code. POSIX
  commands use a separate process group. Shell job control preserves the
  terminal session for prompts. A startup handshake reports the group before
  allowing execution. Cancellation sends SIGTERM, waits 500 ms, then sends
  SIGKILL to the group, including descendants whose parent has exited.
  Windows uses bounded `taskkill /T /F` and waits for command closure.

`test/tailscale.test.mjs` predates these adaptations. Its spawn-failure,
deadline and stderr-diagnostic cases cover the same behaviors as
`packages/tailscale/src/tailscale.test.ts` at 5a906583, using Node child
processes in place of Effect test layers. Serve, exit-node, SSH and
capability cases are not carried over, because Remote has no such commands.
