# AGENTS.md: working in Framerize

Inherited from Notionize, same taste. Read before changing code.

## Glossary

- **account**: one signed-in Framer identity with its own session and device
  state. Keep account, proxy, browser storage and device ownership isolated.
- **workspace / team**: a Framer team (`teamId`, uuid). Credits live here, not
  on the account. Rotation unit.
- **pipeline**: one agent inference turn: mint/refresh AI JWT → Castle token →
  POST `/ai/v3/openai/responses/` → parse SSE → record `X-Framer-AI-Usage`.
- **the fragile layer**: `crates/framer-api`. Everything Framer-specific is there.

## The ways to hurt yourself

1. **Widening the credential boundary.** Sessions stay in the existing private
   session store and account runtime. Never put them in SQLite, logs or Git.
2. **Leaking Framer quirks upward.** Castle, JWT refresh, SSE event names, status
   policy: all stay inside `framer-api`.
3. **Killing healthy accounts.** Only a 401 from `/auth/ai/access-token` itself
   means signed-out. 403 = Castle risk refusal → cool down. 402 = credits gone →
   rotate workspace. Never quarantine on 403/429/5xx.
4. **Changing the default prematurely.** `main` uses the account's own browser.
   Browserless work stays on `browserless` until real delivery, cleanup,
   streaming, tool use and account isolation are demonstrated.
5. **Testing against live state accidentally.** Use focused fixtures for normal
   development. Run live checks only within the user's authorized scope, with
   existing account and rate ownership. Never duplicate paid requests.

## Continue the goal when a step must wait

The user explicitly requires continued work on the original objective.
Never voluntarily stop or mark the whole goal blocked because one selected
step needs authentication, a cooldown window, an external reply or a fix.
Attach the dependency to that step and continue meaningful independent work
within the existing authorization. Do not ask for the same permission again.

Before deciding that no useful work is available, inspect the current source
and checkpoint for remaining implementation, integration and measurement
tasks across delivery, streaming, tools, cleanup, latency, memory and account
isolation. Choose the smallest useful action and carry it through. Recover
context from existing code and state before asking the user to supply it.

Repeated readiness checks, unchanged audits and status messages are not
progress or proof that the whole goal is blocked. A user resume requires a
fresh assessment. Preserve the original objective and all valid evidence.

Do not bypass authentication or runtime ownership, weaken safety checks,
invent results or replace the goal to conceal a wait. Mandatory higher-priority
goal-tool rules still apply if all meaningful authorized work is exhausted;
explain any required transition and retain the exact next action.

## Branches and commits

For authorized legacy-service upgrades, default to graceful draining rather
than waiting for a naturally idle service. Signal the verified serving process
to close admission and finish its existing requests and cleanup. Wait for a
clean exit and an empty service cgroup before switching the release. Never
force termination to meet a deployment deadline. Keep package qualification,
account isolation, configuration, cooldown and compatible rollback guards.
Use `scripts/release_drain.py` for the process-exit checks. Preserve the
installed service settings and all current databases throughout the upgrade.
For qualified binaries whose shutdown joins application ownership and browser
cleanup, bind readiness to the expected PID before signaling and require normal
exit afterward. Preserve damaged diagnostic logs. Never infer completed cleanup
by skipping their malformed records; use the verified in-process ownership proof.

`main` remains browser-based until an explicitly reviewed switch. Keep the
`browser-based` branch as the maintained browser fallback. Develop the native
path on `browserless`; never enable it on `main` just because fixtures pass.
See `docs/architecture.md` and `docs/development-status.md` before choosing work.

Commit each coherent implementation or documentation milestone and push it to
its matching branch. Commit and push before handing off or ending a session.
Do not leave completed work only in scratch snapshots. Untested work may be
committed on the experimental branch when its status is stated clearly.

Review the staged diff and run checks appropriate to the actual change.
Reuse valid saved results for unchanged code; do not repeat broad suites merely
to publish a commit. Record which checks ran and which evidence was reused.
Do not claim fixture success proves live delivery or total memory savings.

Use explicit paths when staging. Keep credentials, account records, browser
profiles, runtime captures, binaries and private receipts out of Git. Preserve
other contributors' dirty work and frozen evidence; use a clean worktree when
needed. Push normally without force. If a push fails, record the local commit
and the failure, then resolve it without discarding remote work.

Keep the status note current with the branch, completed result, known limit
and one exact next action. Only the runtime owner may operate the service.
Branch publication does not authorize an automatic deployment.

## Reference

`reference/analysis/` is the ground truth record: auth flow, AI endpoints, Castle,
credits, desktop decompile, compat mapping. Update it when Framer drifts.
