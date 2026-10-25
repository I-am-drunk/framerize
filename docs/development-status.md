# Development status

The application/library split is being implemented on
`refactor/separate-apps-and-ci-20261025`. It has not been deployed.

Source changes so far:
- One workspace with separate browser and native application targets.
- Native implementation grouped into account, authorization, request, transport and worker modules.
- Shared protocol helpers separated from browser and native execution.
- Normal server routing no longer selects native execution.
- Direct native complete-JSON execution replaces the general provider factory.
- GitHub build jobs target the two applications separately on the build fork.

Validation: `cargo check --locked --workspace --all-targets -j 1` passes.
The retained fixtures compile, but execution in GitHub Actions is still pending.
Synthetic worker fixtures now live in the repository and use Node from PATH.
Earlier runtime results do not qualify these edits.
The native CLI only checks configuration; live activation remains unfinished.

Next: collect GitHub fixture results, package matching host/UI assets,
consolidate host modules and simplify normal request admission.
Durable state and the first live migration remain separate unfinished work.

See [the design](rearchitecture.md). Previous measurements and source results are
preserved in [the history](history/development-status-before-reorganization-20261025.md).
