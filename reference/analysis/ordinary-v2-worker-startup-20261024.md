# Ordinary V2 worker startup check

On 24 October 2026, `scripts/check_v2_worker_startup.rs` passed against the
unchanged worker owner linked into native package `6012de1c`. All nine owner
implementation files match the current integration source. Existing owner
qualification was reused; the new standalone check was compiled separately.

The check sends an invalid route and URL with an untrusted test marker.
Target validation rejects these before constructing the saved-device object,
loading the SDK factory, creating a DOM or entering a mint operation. It requires
the real V2 decoder's `input` refusal with no operation or device failure,
then awaits the original completion handle and verifies child absence.
This exercises process start, static entry imports, descriptor exchange,
failure decoding and joined cleanup. It does not qualify valid seed transfer,
lazy SDK imports, successful minting, inference, streaming or account isolation.

Compilation passed with a 115,335,168-byte peak under a 512 MiB limit.
Runtime passed with a 25,579,520-byte peak under 128 MiB. Both retained zero
swap, 32 tasks and bounded deadlines. The runtime reported no memory or task
limit events. Its worker completed in approximately 664 ms.

The runtime used private PID and network namespaces, masked host homes and
runtime directories, and read-only inputs. The private library copy was mounted
at the worker's original required path. All 1,917 pins match before and after.
The child reported namespace PID 3; that is not a host PID. Captured outer
processes are absent, both units/groups are collected, and the shared lock is
free. The external snapshot missed transient descendants, so it does not
establish exhaustive process capture. Runtime receipt prefix: `50270d74`.

No service, account, genuine seed, SDK mint or HTTP request was used. Normal
rollout remains separately authorized, pending the bound terminal authentication
and a current quiet window with positive endpoint checks. The complete native
mint and inference outcome remains unverified.
