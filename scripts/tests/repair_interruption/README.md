# Repair interruption acceptance fixtures

These are the exact source bytes qualified for the R9 acceptance correction.
`run_checks.py` runs the 15 names in `selection.json`; it refuses selected skips.
The retained tests supply an inert `control_http` shim. This directory is not a
standalone live requester and contains no account or service configuration.

The normal browser path buffers a complete upstream response before emitting
that response's generated items. A check requiring generated text while also
requiring zero completed upstream posts cannot pass on that path.

`sequence.run_pair(..., cancellation_boundary='request_started')` instead waits
for an observed start and valid open downstream response, then disconnects.
It retains the existing completed-post refusal, HEAD recheck, same-request
joined cleanup, exact attempt counts, and subsequent normal delivery checks.
The second response must have actual delta text, terminal completion and EOF.
The old `generated_text` default remains for compatibility and regression tests;
it is not selected for the buffered browser route's R9 run.

Start-log presence and absent completion logs do not prove an active fetch.
The report explicitly records `upstream_cancellation_proven=False`. R9 observed
the disconnected request finishing successfully, with its owned work joined
and released before the second request. It did not establish upstream abortion
or a live disconnect after generated text. Synthetic after-text socket coverage
is distinct from that live result.

All 15 selected cases passed under the retained private-network Python harness,
128 MiB memory, zero swap, 32 tasks, one CPU, deadline and shared execution lock.
The other retained test bodies are support or historical coverage, not newly
claimed results. See `qualification.json` for source hashes and collected evidence.
