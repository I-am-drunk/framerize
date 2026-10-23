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

Next: integrate this real graphics implementation into a separate synthetic
SDK experiment while preserving the current worker's lifetime, resource and
cleanup controls. No SDK, token, authenticated request or Framer delivery was
tested by this fixture. SharedWorker, WebGL and other missing browser APIs
remain unsupported; Castle still does not document JSDOM as a supported runtime.
