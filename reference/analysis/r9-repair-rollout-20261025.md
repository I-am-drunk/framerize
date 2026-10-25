# R9 repair deployment and completed checks

The normal service on port 8787 now runs qualified source `93dad965`, binary
`f70458d364ec624006b5dc968fda49c06ab22aa65673c56fc43b87d7d223aba3`, with selected
host `f94e2d48f7c975c78071ee1138bc4dda14ed46acf97cc320e2bdbde321306714`.
One controlled switch completed. Both acceptance requests passed on one service
generation, using an existing normal account. No signup or account migration
was performed. All eleven account lifecycles and unselected records were preserved.

The first downstream connection was closed after its request start was recorded,
before generated text was observed. Its upstream post subsequently completed
successfully. The original producer joined and its child and root registrations
were released with cleanup confirmed. The second request then delivered HTTP
200, two delta-text characters, one terminal completion and EOF on the same
service generation. This verifies retained cleanup and subsequent delivery.
It does not prove an aborted active fetch or live interruption after generated text.

R8 exposed an incompatible acceptance condition: this browser path consumes
`r.text()` before the sidecar reply, Rust body parse and generated-items callback.
Text therefore requires a completed supplying upstream response. A continuation
may follow, but absent completion logs alone cannot establish active work.
The revised explicit `request_started` mode has 15 passing targeted tests.
The legacy predicate, cleanup guards and exact two-request limit remain covered.
Source fixtures are in `scripts/tests/repair_interruption/`.

Final verification caught `Restart=no` in the candidate deployment drop-in.
After successful acceptance, a narrow steady-state drop-in restored
`Restart=always` with daemon reload only. No second restart or switch occurred.
The candidate remains running; all controller/authentication helpers are gone,
temporary ingress restrictions were removed and the shared execution lock is free.
All 124 source pins and fifteen packaged files matched their bindings.

Collection: `991a68f427b1730b468a9db32c6c413788ac976e1328c1599a697239237c635c`.
Qualification: `8a5f05814ef05fca971a1557f06b1715bd13cc53daa2e1839388bb7de6e93082`.
Restart finalization: `412903c5dfb8c4d1bb1e11b420c4b4e7384af21d6320315940ddbf29cce30035`.

This is the normal-browser repair rollout. It does not qualify the separate
native V3 route, later control-transport changes, host-global OOM recurrence,
or every historical browser network failure. Browserless development and its
memory, latency and account-isolation acceptance remain separate unfinished work.
Future native tests use the user-authorized existing account pool with its
current identity, rate, egress and cleanup checks; new signup is not a prerequisite.
