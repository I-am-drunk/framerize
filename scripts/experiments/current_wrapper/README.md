# Current Castle wrapper qualification

The October 24 public app selects `chunk-VPFQZMNG.mjs`. Its token generator
is export `b`; export `a` only initializes. `sdk_loader.mjs` now uses the
explicit `WRAPPER_MINT_EXPORT` pin. The checked-in historical pins retain `a`.
The separately staged candidate selects `b` and all 14 unmodified public
modules recorded in `module-graph.json`.

`qualification.json` binds seven passing loader cases, the exact old-loader
control (three expected failures, four passes), and one real synthetic SDK
run. It generated and discarded a token, joined the worker, and peaked at
122,941,440 bytes under a 128 MiB group cap with zero swap or memory events.
Explicit cancellation and owner-drop cases also passed after observing the
worker running. They do not identify its SDK phase or prove live interruption.

The owner directly pins six SDK files and the complete graph manifest, within
its unchanged 64-file limit. The enclosing runner and child loader verify all
14 module files before SDK evaluation. No dependency check is skipped.
Both units and groups were absent after every saved run.

`qualification.json` retains the original synthetic result. The subsequent
guarded update selected this bundle only on the isolated instance, preserving
its application binary and routing. `live-result.json` records one POST and
one upstream send, a complete Framer security 403 after 5.24 seconds to
headers, confirmed cleanup and real history restoration. No second request
ran. The protected browser service remained unchanged. Native lifetime peak
was 140,722,176 bytes with zero sampled swap; this is refused-request evidence,
not successful delivery or live interruption.
