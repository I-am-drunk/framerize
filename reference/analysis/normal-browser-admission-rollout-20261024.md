# Normal-browser admission rollout preparation

The normal-route candidate is source `95f29867`, using the qualified executable
SHA256 `cf0ce734a4d1e932458055dadf2206e56d4162b5df34519fa7754e2229ab0a2c`.
Its startup, API, local/static, legacy and capacity results remain retained.
No source or package dependency requires ordinary V2 before this rollout.

Runtime owns a staged eight-file package: that executable and seven exact
Python support sources. The proposed launch adds `--browser-request-ownership`
and an explicit packaged `--sidecar-script`, retaining normal Camoufox routing.
The existing Python runtime, memory browser and addon are statically bound.
This does not qualify runtime imports or actual browser interaction.

The private rollout proposal preserves the current executable, support and
unit configuration. It changes only a new service drop-in and uses a separate
release directory. Rollback removes that drop-in after safe shutdown; it never
rewinds account, session, cooldown or profile state. The protected normal
service has not changed. Explicit approval and a safe maintenance window are
required before any service operation or normal-route test.

Full acceptance still needs an interrupted stream and a complete subsequent
stream on one service generation. A source check identifies an observation
limit: `ai.chain` is emitted before the producer returns, and the later cleanup
completion and gate registration release have no correlated production record.
Pool residency is not gate ownership. Bind direct payload-free completion
evidence before claiming that part of acceptance; a second successful request
alone cannot prove it. This is separate from V2 and not a new functional-test
failure. No new test, application execution or live request was performed.
