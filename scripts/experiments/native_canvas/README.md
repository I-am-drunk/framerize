# Native canvas qualification

This experimental fixture uses JSDOM 26.1.0 with node-canvas 3.2.3 and its
published Linux x64 N-API 7 binary. The application does not load it.
The existing worker, dependency directory and running services are unchanged.

The fixture drew real pixels, read them back and round-tripped a generated
PNG through an image. It rejected external URLs, local files, SVG, malformed
PNG and an excessive image dimension. The loader accepts at most four PNGs,
each at most 65536 encoded bytes and 262144 pixels. Other resource loading
remains disabled. These are fixture limits, not new production defaults.

Unmodified JSDOM returned its cached 2D context for a later WebGL request.
The saved first run failed on that assertion. The four-line dependency patch
returns null for context types the native backend does not implement. The
corrected fixture also checks unsupported contexts before creating 2D.

The corrected run passed under Node 24.13.0, with a 96 MiB JavaScript heap,
128 MiB group limit, no swap, 32-task limit and private network/filesystem
namespaces. Group peak was 73601024 bytes and ten tasks; no memory events
were recorded. Child completion and group/unit absence were verified.
This is one synthetic graphics sample, not a memory saving or SDK measurement.

To stage the fixture, place its exact dependencies in an adjacent node_modules
directory, apply jsdom-context.patch to that private JSDOM copy and use the
bounded qualification launcher. qualification.json binds the tested source,
dependency artifacts and saved result. Package lifecycle scripts were not run;
the published source and native archives were verified before extraction.

A subsequent synthetic SDK integration now passes. The existing compiled
JITless owner ran the unchanged SDK with the qualified graphics dependencies,
original private token pipe, synthetic device and original cleanup checks.
The first run minted successfully but failed the overall resource check:
it reached 128 MiB and recorded 53 memory-limit events, with no OOM or swap.
That source and failed result remain preserved.

The second run changes only file verification among worker modules. It uses
the existing 64 KiB hashing-buffer implementation instead of allocating each
entire native library. It passes at 84398080 bytes of group peak memory and
12 tasks, with zero memory events or swap. Two local PNGs were loaded; JSDOM
errors and console warnings/errors were zero. One unclassified process warning
remains. V3 selection, device preservation, token handoff/discard, worker join
and absence all passed. The outer unit and group were absent afterward.

sdk-worker.patch reconstructs the experimental worker delta with
`git apply --unidiff-zero`. It includes
diagnostic output to the fixture's /out directory and must not be applied to
the application worker. Its separate dependency overlay binds JSDOM, canvas,
native libraries and the copied context patch with an explicit file-hash
manifest. This is not an npm lockfile. Machine-specific dependency paths and
pins are outside this portable patch. sdk-worker-qualification.json binds the
saved result and records its limits. No Rust rebuild was needed.

No authenticated request, application deployment or complete-service memory
measurement ran. SharedWorker and WebGL remain absent; Castle still does not
document JSDOM as supported. Next: package this qualified dependency/worker
variant for the isolated instance while retaining account history and browser
protection, then qualify real delivery under the existing request controls.
