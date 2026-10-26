# Kernel and application updates

Investigation on `refactor/kernel-hot-swap-20261026`, based on qualified
`c8371bf`. This branch is not a live release. The existing activation command
still stops and restarts the application; it does not preserve active streams.

The browser application currently owns its HTTP listener, account admission,
private session-store lock, concurrency history, state database and browser
bridge. The Python host owns browser processes, page work and the rate journal.
The native application has its own request, session, device and worker owners.
Starting a second complete application would contend with these owners.

Some ownership is currently implicit: `core/rotation.rs` has process-global
workspace saturation and selection cursors, while `api/auth.rs` has a shared
session-keyed token cache. Those cannot become independent worker-local state
without changing behavior. `Session` contains the private cookie and has no
serialization implementation. A process adapter must use request-bound opaque
handles rather than serialize the current credential-bearing port objects.

An update boundary must retain one state owner and pin each accepted request
to its application generation through response delivery and external cleanup.
A response body ending is not proof that browser or worker cleanup completed.
Unconfirmed cleanup must retain the current admission fence.

The intended first proof is an offline generation transition: A continues an
accepted stream while ready B accepts new requests. Failed readiness leaves A
active. Rollback changes future admission without moving accepted work.

The production boundary is still under design review. A neutral kernel,
persistent account runtime and replaceable request-policy workers would keep
frequently changing API policy out of the stable kernel. A generic proxy alone
does not solve state ownership. Rust dynamic libraries add ABI and destructor
lifetime constraints; they are not the default approach.

Every transition needs explicit protocol and state compatibility, bounded
transport, cancellation, process-exit observation and retained rollback state.
Kernel upgrades and incompatible state migrations require a separate drain.
Process isolation does not imply survival of a worker crash or forced kill.

Pending source fixes remain dependencies. In particular, A owns sidecar page
acquisition and page-use cancellation. This overhaul must preserve their
bookkeeping and finalizer behavior without duplicating those changes.

CI currently runs on pushes to the `diddletime/framerize` build fork. It builds
both applications with locked dependencies and runs offline/package fixtures.
Upstream PRs do not trigger a duplicate build. A successful local fixture run
does not establish hosted CI success or live account acceptance.

## Reference evidence

These historical revisions were inspected, rather than inferring quality from
repository age. Current documentation is listed separately.

- [Tokio 1.14, 2021-11-16](https://github.com/tokio-rs/tokio/tree/884a9a4b1800219dee696ce9bf8eb8fe93b92f71): `sync/mpsc` and its tests distinguish closing admission from consuming remaining messages. Bound bytes as well as message count; a channel cannot attest browser cleanup.
- [Redis 6.2.6, 2021-10-04](https://github.com/redis/redis/tree/4930d19e70c391750479951022e207e19111eb55): command admission and `tests/unit/quit.tcl` test that buffered work after QUIT is not executed. Its single-threaded command model should not become a global lock around long streams.
- [ripgrep 13, 2021-06-12](https://github.com/BurntSushi/ripgrep/tree/af6b6c543b224d348a8876f0c06245d9ea7929c5): executable glue composes focused libraries. It supplies a code organization example, not a service update protocol.
- [Cargo 0.57, 2021-10-04](https://github.com/rust-lang/cargo/tree/4ed5d137baff5eccf1bae5a7b2ae4b57efad4a7d): the integration suite groups concurrency, process death and compatibility behavior. Test organization alone is not proof of coverage.
- [nginx 1.20.1, 2021-05-25](https://github.com/nginx/nginx/tree/98a892514206ee63d418d9f355c92b6d4ce6113a): process-cycle reload retains the old configuration on failure and drains old workers. Its 100 ms startup pause is insufficient readiness evidence for Framerize.
- [Hyper 0.14.15, 2021-11-16](https://github.com/hyperium/hyper/tree/d0b1d9ed3a10013ab356bc7d9b283e179857a672): shutdown and drain watchers separate notification from completed futures. External resource cleanup needs additional ownership.

The current [Tokio shutdown guide](https://tokio.rs/tokio/topics/shutdown)
separates detection, notification and waiting. The
[nginx control guide](https://nginx.org/en/docs/control.html) distinguishes
configuration reload from executable upgrade. Both were read on 2026-10-26.
[OTP 24 release handling](https://www.erlang.org/docs/24/design_principles/release_handling)
requires explicit upgrade/downgrade paths and warns about suspension windows;
Rust has no equivalent VM hot-code contract. [SQLite testing](https://www.sqlite.org/testing.html)
motivates targeted I/O, crash and compound-failure tests, not copying its scale.

[matklad's 2021 architecture guide](https://matklad.github.io/2021/02/06/ARCHITECTURE.md.html)
supports a short boundary map separate from historical evidence.
[Spolsky's 2000 rewrite essay](https://www.joelonsoftware.com/2000/04/06/things-you-should-never-do-part-i/)
supports retaining accumulated behavior through incremental replacement.
Neither article establishes correctness of a proposed Framerize design.
