# Isolated SDK snapshot comparison

## Direct input comparison, 2026-10-25

Schema 3 also copies the actual value array immediately before the original
serializer consumes it. The native checker uses those captured values directly,
rather than deriving its input from the serialized bytes. It first checks their
serialization, then rebuilds the five existing collector positions and checks
serialization and the complete encoding again with the same observed timestamp.

One new isolated synthetic SDK operation passed: 765 positions, 787 values,
5,412 serialized bytes and a 5,297-byte final output with the same SHA-256.
The unavailable network record was also rebuilt natively. Three inert capture
tests pass. Both isolated processes exited, source pins stayed unchanged and
the SDK's owned descendants were absent. The existing sandbox limits apply.

The capture preserves primitive values and plain arrays, including raw UTF-16
and negative zero. It refuses objects, accessors, inherited indexed values,
proxies, cycles and oversized inputs. It does not run conversion hooks.
These reads can affect timing; the comparison uses the timestamp from that
same instrumented operation. No raw values or output token are published.

This proves the direct input comparison for one synthetic operation. It does
not show that every configured collector finished, expose a production
collect-only export, connect native encoding to serving, or measure savings.
The serving loader still evaluates the pinned SDK without these hooks.
Schemas 1 and 2 remain readable; their results below retain their original scope.

The pinned SDK `3337eba7` ran once with synthetic context in the existing
JSDOM worker. Test-only hooks copied its serialized bytes and recorded the
timestamp supplied to its final encoder. The SDK returned normally; its
output was hashed and discarded. No account or upstream request was used.

The snapshot contains 765 positions, 787 values including nested values,
and 5,411 serialized bytes. Python reconstructed normalized values; native
serialization reproduced those bytes. `encode_collected` then matched the SHA-256 and
5,293-byte length as the instrumented SDK. One test passed, zero skipped.
This checks normalized representation and encoding for this observation.
It does not recover pre-serialization object behavior or prove collection
equivalence, browser equivalence, live acceptance or memory savings.

The first launcher stopped before SDK evaluation because it omitted the
worker syscall filter. An inert replay confirmed the other four runtime
predicates passed. The corrected launcher reused the exact production
filter from `worker/process/worker_sandbox.rs`. Socket, socketpair, io_uring
and fork denials were checked before SDK startup. The runtime
guard was unchanged. Both failed and corrected evidence are retained.

The SDK run used private network/PID namespaces, hidden home/runtime paths,
read-only software, a 128 MiB limit, zero swap, 32 tasks and a 40-second
deadline. The native comparison used the existing isolated 512 MiB harness.
Both runs joined and left no owned process; source bindings were unchanged.

Tools: `scripts/castle_v3_snapshot.mjs` supplies the observation hooks;
`scripts/castle_v3_snapshot_loader.mjs` is mounted as `/work/sdk_loader.mjs`
over the pinned worker bundle. Mount the recorder, its
`scripts/castle_v3_snapshot_values.mjs` companion and
`scripts/check_v3_sdk_snapshot.mjs` under `/probe`. Run only with the above
sandbox and an empty writable `/out`; the loader is not a production change.
In the isolated compiler view, run `python3 -I -B scripts/check_v3_sdk_snapshot.py
/snapshot.json /out/comparison.json`. Keep snapshots and generated values
out of Git. This SDK experiment is not part of ordinary CI.

The current schema also captures two collector outcomes for the separate
[collector-input comparison](castle-v3-collected-inputs.md). The checker
still accepts the original byte-only snapshot schema described above.

The optional `--network-unavailable` comparison reconstructs the captured
network record with native `unavailable_network_record()` before applying
its consumers. It first requires full serialized record equality; an
unexpected nonempty or differently hashed record fails the comparison.
The saved synthetic capture passes: the complete 5,412-byte serialization
and 5,297-byte final output still match. This reuses the existing capture
and does not run the SDK again. It establishes fallback reconstruction for
that observation, not native network availability detection.
