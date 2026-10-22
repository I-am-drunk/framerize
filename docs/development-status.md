# Development status

Updated 2026-10-22. The normal service remains browser-based. Publishing these
branches does not change the running service.

| Branch | Purpose | State |
| --- | --- | --- |
| `main` | Normal browser-based path | Verified browser source restored to Git |
| `browser-based` | Maintained browser fallback | Same browser baseline |
| `browserless` | Native request path and SDK worker | Experimental; integration remains |

## Browser baseline

The source comes from the saved `pool-lifecycle-validation-r3` snapshot.
All 89 source hashes were checked before import. The accepted saved lifecycle
results cover 15 phase sets, 197 Rust cases and 27 separately reused Python
cases. Those results were reused, not rerun for this publication.

The deployed binary has SHA-256
`6fb7795117f1ec7c49f06d9453c0466b6468bbc257326715bb3bccf90587ea67`.
The original preparation hash is
`ac8b3c00d8a27e7f81836d436a1a153b157cd3379c538185988cf3566cdf6911`.
The accepted saved-results review hash is
`32abcb23db4f6c1903db45225b6b03b914c62995f357274ee1d5230b484b6fdd`.

The runtime cooldown-restoration overlay is separate from the embedded base
Python host. Preserve that overlay and the original cooldown timestamps in
any subsequent deployment.

The published `scripts/prepare_browser_runtime.py` reproduces the deployed
sidecar and support-manifest hashes exactly from tracked source. This was
checked offline during publication, without importing or running the host.

## Browserless progress and limits

The earlier serving foundation passed 882 component cases. Later focused
routing validation passed 49 API cases and 38 application cases, with 67
application cases filtered out. The routing binary is built and undeployed.
These results do not establish a new full-suite pass for the later source.

The routing worker passed 19 cases. A separate real SDK mint succeeded with
the token discarded; it did not load account state or send an upstream
inference request. Neither result proves end-to-end client delivery or the
total-memory target.

The saved-device worker and shared device/rate admission components have
accepted source reviews. Their new 28-case and 23-case suites are still unrun
at this checkpoint. The host/application connection is unfinished.

Next action, owned by the primary implementation agent: connect the existing
restored host authority to the native request lifetime and protocol-v2 worker,
validate that connection, then run one bounded real request. Keep this work on
`browserless`, commit each coherent step and retain normal browser serving.
