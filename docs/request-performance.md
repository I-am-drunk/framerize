# Request performance checks

`npm run test:performance` sends synthetic requests through real server and
Remote listeners on loopback. It needs no account or network provider.
It checks request forwarding and bounded work; it does not measure provider
latency, validate live model support or prove billing behavior.

## Run locally

1. Install the development tools with `npm ci --ignore-scripts`.
2. Build the tested runtime with `npm run build`.
3. Run the workload and save its measurements:

```sh
FRAMERIZE_PERFORMANCE_REPORT=/tmp/request-performance.json npm run test:performance
```

The report path is optional. Set `FRAMERIZE_PERFORMANCE_REPORT` using your
shell's environment syntax. Rebuild after a runtime source change so the
measurements use current code.

## What it checks

| Client route | Upstream route |
| --- | --- |
| `/openai/v1/responses` | `/ai/v3/responses/` |
| `/anthropic/v1/messages` | `/ai/v3/messages/` |

Sixteen cases cover both routes, direct server and Remote use, 1 KiB and
256 KiB text payloads, and the shared Unicode, history, tool and attachment
corpus. Model labels are opaque synthetic values. Each case warms up twice,
then measures sixteen requests in groups of four. The upstream holds each
group until all four requests arrive, so serialized forwarding fails.

Every upstream request must preserve the model and options and use
`stream: false`. These performance cases check completed JSON responses.
`npm run test:custom-routes` separately checks client streaming, omitted
stream options, cancellation isolation, abandoned uploads and body limits.

The workload requires one upstream request, credential lookup and token mint
per client request, unique request identities and exact response bytes.
Cleanup must release sockets and abort listeners without forced closure.
The upstream body may add at most 1,024 bytes to the serialized client body.
The initial measured maximum was 605 bytes. This budget catches a duplicated
request or an expanded prompt preview without relying on machine speed.
The fixture counts bytes received by the upstream, including JSON whitespace.

## Read the report

CI uploads `request-performance-node-<node>-<sha>` with the JSON report on
Node 22.23.4 and 24.13.0. The source SHA identifies the tested checkout,
which is GitHub's merge commit for a pull request. The report also records
whether tracked or untracked source changes were present.

Each case records request counts, request and response bytes, envelope
overhead, elapsed time, p50/p95/maximum latency and requests per second.
Byte totals include both warmup requests. Latency and throughput cover only
the sixteen measured requests. Compare timing only on comparable machines
and runtimes. Timing is diagnostic; correctness, byte limits, concurrency
and bounded completion determine the test result.

Artifacts can be partial after a failure. Compare `cases.length` with
`expectedCases` and use the job result to determine whether assertions and
cleanup passed. A JSON artifact alone is not a passing test result.

## CI selection

The `performance` job runs for request-path changes and its own test and
workflow changes. Documentation-only changes and explicitly listed unrelated
CLI files skip it. A dependency test checks that skipped files are outside
the measured runtime imports. Unknown paths, unresolved diffs, pushes and
manual full runs select it. Missing gate outputs cannot authorize a skip.

The required `Check` job includes performance failures and cancellations.
Existing source, package and native checks keep their selection rules.
PR package previews still run for documentation-only changes.
