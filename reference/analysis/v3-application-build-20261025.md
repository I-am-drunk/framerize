# V3 diagnostic application build, 25 October 2026

The worker library at source `191dabe8` compiled successfully under 512 MiB,
no swap, 32 tasks and one CPU. Its SHA256 is
`b86858aa8c495b2a748f76a763729fd5f8b7e6792ebfd3a3c52c89c2ea824195`.
The isolated job is collected, with unchanged inputs and no limit events.
This includes the decoder covered by the 31 accepted diagnostic cases.

The first application link failed with nine E0463 crate-resolution errors. Its direct
dependencies were pinned and mounted, but the older build manifest omitted
transitive libraries that had been visible through its broader filesystem view.
The stricter isolated build exposed that preparation gap. Retained Cargo
dependency records and embedded crate identities identified 120 missing files:
108 rlibs and 12 proc-macro libraries. Their closed dependency set was added
without changing source or rebuilding the five retained application libraries.

A second attempt resolved those errors and reached the linker, exposing two
missing toolchain executables in the masked sysroot. A fresh third attempt
added only the same toolchain's `rust-lld` and `gcc-ld/ld.lld` and passed.
The resulting 29,084,520-byte executable has SHA256
`7a10ec27403a2f50cdf6d67468362c64607a7a03e1417794b07770d62c1183e3`.
It linked in 14.2 seconds under 768 MiB, zero swap, 32 tasks and one CPU.
Peak memory was 466,911,232 bytes. All 797 input hashes remained unchanged;
all 44 source paths in dep-info were pinned. No limit events occurred.

All attempts are collected. Known processes, units and cgroups are absent,
and the shared lock is free. This retains the accepted worker library and
31 diagnostic passes. The executable and SDK have not been launched.

The prepared V3 configuration retains 32 direct pins and 1,259 source and
dependency hashes, all checked. The existing native host and 19 support files
also match their retained bindings; all 16 generation inputs match current
source. The older application binary is excluded from the new assembly.

The normal repair candidate is unchanged and remains independently authorized.
Its deployment still requires active calls to finish. Private V3 account
binding and real upstream acceptance remain unfinished. Separate authority
source `c8a5491a` is unqualified and is not included in this binary. Its changed
core grant metadata requires rebuilding the affected library graph before
qualifying the matching host and client.
