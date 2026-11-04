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
- `skills/ship-framerize-pr/SKILL.md` is Framerize's own. Agents review their own
  PR with an independent subagent, then merge or close it before the next PR.

T3-specific WebSocket/Effect architecture, browser-panel commands, desktop/mobile
tooling, account-data copying, vouch/exemption lists, Macroscope labels, model bans
and automatic closure rules are not carried over. Framerize uses its existing
TypeScript packages, synthetic HTTP fixtures and Codex review integration.
