# T3 Code attribution

The repository guidance adapts T3 Code by T3 Tools Inc. under the MIT License.
Source revision: `7c22e24e3fa1a3bb3a95f27aea000067f198fe7f` in
[pingdotgg/t3code](https://github.com/pingdotgg/t3code/tree/7c22e24e3fa1a3bb3a95f27aea000067f198fe7f).
The full [MIT notice](../framerize-remote/T3-LICENSE) is retained in this repository.

- `AGENTS.md`, `CONTRIBUTING.md` and the PR template adapt the matching upstream
  files: simple design, coherent scope, focused verification and useful documentation.
- `skills/review-framerize/SKILL.md` adapts evidence, trusted-policy and current-head
  checks from `.agents/skills/contribution-triage/SKILL.md`. It adds Framerize source
  review and does not implement T3's automated contribution moderation.
- `skills/test-framerize/SKILL.md` adapts isolated state, meaningful fixtures,
  owned processes and integrated verification from `.agents/skills/test-t3-app/SKILL.md`.

T3-specific WebSocket/Effect architecture, browser-panel commands, desktop/mobile
tooling, account-data copying, vouch/exemption lists, Macroscope labels, model bans
and automatic closure rules are not carried over. Framerize uses its existing
TypeScript packages, synthetic HTTP fixtures and Codex review integration.

## CI change selection

These files adapt T3 Code at commit `5a906583fed3fa41dc806f2a19bd9d095537ed3e`
under the same MIT License:

- `.github/scripts/ci-changes.cjs` adapts `.github/scripts/ci-changes.cjs`:
  `readWorkspace`, `ownersOf`, `decide`, `main`, the inert-path rule, the
  dependents walk and fail-open selection. It reads npm workspaces and
  `file:` dependencies instead of pnpm, and reports `source`, `packages` and
  `server` gates. Rust, mobile, web, release-smoke, test-package lists and
  T3's extra-owner tables are omitted.
  `BUILD_ONLY_DEPS` has a different role here: T3 excludes those dependency
  edges from its affected-package walk. Framerize adds Core as a Server
  dependency because Server bundles Core, so Core changes also select Server.
- `.github/actions/ci-changes/action.yml` adapts the matching composite action.
  Pushes and manual runs pass `--full` without fetching a base commit.
- `.github/workflows/ci.yml` adapts the `changes` job and the required `Check`
  job's skip rule. The existing source, package and server workflows remain
  the gate implementations, unchanged apart from their triggers.
- `test/ci-changes.test.mjs` adapts the inert-docs case from
  `scripts/check-changed.test.ts`. Its dependency, full-selection and `Check`
  cases are Framerize tests of the adapted logic.

## Pull request previews

The preview workflows and scripts adapt T3 Code at commit
`fbe5df2d4b630d13adc8fe2d38cab354e6d66d67` under the same MIT License:

- `.github/scripts/preview-version.cjs` adapts the prerelease identity approach
  in `scripts/resolve-nightly-release.ts` for Framerize npm package versions.
- `.github/workflows/pr-previews.yml` and its publisher adapt the separation of
  unprivileged builds and trusted publication in
  `.github/workflows/desktop-macos-preview-publish.yml` and its single PR comment.
- `.github/scripts/stage-pr-preview.py` adapts the bounded ZIP validation in
  `.github/scripts/stage-preview-bundle.py`. Framerize permits two npm tarballs and two
  JSON manifests, not a desktop application bundle.

Framerize retains its own package consumer checks, separate PR-head and merge
identities, immutable asset hashes and foreground isolation instructions.
No T3 desktop updater, signing, Electron packaging or npm publication is added.
