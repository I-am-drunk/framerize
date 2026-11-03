# Claude checkpoint candidate

The private body capture found eleven adaptive `claude-opus-5` requests
ending with an observed upstream HTTP 504 after 60 to 62 seconds. Each
allowed 64,000 output tokens. Successful requests used the same model,
large histories and 98 tools. Request size alone does not explain failure.

The alpha.24 candidate starts supported Opus 5 reasoning at 600 output
tokens and adapts completed checkpoints toward a 45-second window, capped
at 1,200 tokens. It preserves signed and redacted blocks, requested effort,
Fast mode and the total output allowance. A completed truncated tool call
can retain its preceding checkpoint and request smaller complete calls.
Incomplete tool inputs are never published. Unknown completion, provider
refusals and server-tool truncation do not enter that recovery path.

Checkpoint generation currently applies only to Opus 5. Other adaptive
models keep their existing behavior until separately qualified.

The corrected alpha.24 test completed an eight-section answer in four
upstream chunks, each below 22 seconds. Its client follow-up failed with
HTTP 400: signed thinking blocks could not be modified in the latest
assistant message. Concatenating chunks had lost their original turn
boundaries even though the block values were unchanged.

Alpha.25 restores those original turns before forwarding client history.
A private store records content hashes, block offsets and continuation
kinds, without client prompts, signatures or credentials. Records survive
restart, use a 1 MiB SQLite cache and have a 128 MiB disk limit. Existing
records are retained; storage failure stops publication of an unrestorable
checkpoint. Real package qualification remains required before activation.
All 149 affected Messages tests pass. The failed alpha.24 qualification
shut down normally in 4.020 seconds, left no owned processes, retained all
500 credits and peaked at 1,747,660,800 bytes without memory-limit events.

The first alpha.24 real test served Astra through Responses in 20.566 seconds,
including browser startup. A subsequent Claude Responses request received an
unexplained upstream HTTP 403. It did not enter the Messages checkpoint path.
The saved 600-second refusal cooldown remains in force until 05:57:22 UTC.
Normal shutdown took 11.673 seconds with no owned processes remaining, no
memory-limit events, and all 500 workspace credits intact. Peak cgroup memory
was 1,717,903,360 bytes.

Both the outer model and nested request model in the affected production
captures are `claude-opus-5`, which matches the checkpoint selector. OpenAI
clients intentionally retain their native Responses route; that route's 403
does not explain the Messages failures. The corrected qualification exercises
buffered and streamed Messages continuation, combined client follow-up,
complete tool inputs, repeated requests and normal shutdown on port 8792.

## Browser startup memory

The isolated alpha.23 test was killed at its 2 GiB cgroup limit before a
response. The kernel attributed 1,024,892 KiB of anonymous RSS to the
browser's WebExtensions process. The bundled optional ad blocker caused
an avoidable startup allocation. Alpha.24 excludes that default extension
through Camoufox's supported option. Explicit prepared addons stay intact.

A matched test opened two signed-out Framer login pages and held them for
60 seconds, with the same browser, 2 GiB memory limit and no swap. Peak
sampled cgroup memory fell from 1,561,464,832 to 1,084,952,576 bytes, with
samples every 500 milliseconds. The final ten
seconds averaged 1,020,995,584 and 1,001,069,363 bytes. This primarily fixes
a startup spike. It is not evidence of a similar reduction in steady use.
Both tests closed normally. Neither used an account or sent inference.

The failed account test kept all 500 credits. Its processes were confirmed
absent and normal recovery preserved every device and request-history
entry. The resulting 600-second hold ends at 05:45:22 UTC on November 3.
Production remains alpha.21, PID 654353, with no restart during this work.
