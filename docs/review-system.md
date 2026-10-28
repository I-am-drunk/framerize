# Review system decision

Use self-hosted PR-Agent for the first Framerize review pilot. Its inspected
source supports an arbitrary OpenAI-compatible base URL and exact model ID.
This establishes compatibility potential, not that it finds more bugs than
CodeRabbit or Macroscope. No live router qualification has run.

Pin the candidate to PR-Agent `0.49.0`, source
`efdde4afe14aa401a3889012c7f3e9ef3081a430`. Its CLI image is
`pragent/pr-agent@sha256:10a8893d5e081be1a5ae825843d9fc14f8e8d59d8a4f61d67c1775415156079f`.
Verify the official attestation before installing it. Docker tag metadata
alone is not an installed-image or provenance check.

Configure `openai/gpt-6-astra-ultrafast`, disable fallback models and load
settings only from trusted deployment configuration. Check the exact wire
model, reasoning effort, token limits, timeout and response parsing against
a fake provider first. Route no review through an unqualified Anthropic path.

The initial worker produces local review artifacts without a GitHub write
token. It reads PR diffs as data and never executes PR code. Use a fresh
isolated process with bounded time and concurrency. Keep production account
files and deployment credentials unavailable. A separate publisher must
validate findings and current PR head before posting comments. It must never
approve, merge, or deploy a change.

T3 Code provides the reference for the aggregate `Check` job, path-specific
review instructions, evidence-bearing PRs and separate release channels.
Its CodeRabbit and Macroscope installations are real, but those commercial
products do not establish an arbitrary-router option for this deployment.
Macroscope explicitly excludes custom endpoints. CodeRabbit documents
self-hosting for Enterprise deployments with at least 500 seats.

The existing Framerize CodeRabbit installation did not review PR #13. Its
check reported `SKIPPED` because automatic reviews exclude non-default base
branches. The bot also reported the Free plan, which supplies summaries and
walkthroughs rather than comprehensive line review. A bot check appearing
on a PR is therefore not evidence that its changes were reviewed.

Sources inspected on 2026-10-27:
- [PR-Agent model configuration](https://github.com/The-PR-Agent/pr-agent/blob/efdde4afe14aa401a3889012c7f3e9ef3081a430/docs/docs/usage-guide/changing_a_model.md)
- [T3 Code CI](https://github.com/pingdotgg/t3code/blob/5e9d56416f6b395095c772c597e79cd5f94e3052/.github/workflows/ci.yml)
- [T3 Code review configuration](https://github.com/pingdotgg/t3code/blob/5e9d56416f6b395095c772c597e79cd5f94e3052/.coderabbit.config.ts)
- [T3 Code releases](https://github.com/pingdotgg/t3code/blob/5e9d56416f6b395095c772c597e79cd5f94e3052/.github/workflows/release.yml)
