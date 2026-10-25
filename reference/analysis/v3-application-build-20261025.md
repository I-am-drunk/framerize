# V3 diagnostic application build, 25 October 2026

The worker library at source `191dabe8` compiled successfully under 512 MiB,
no swap, 32 tasks and one CPU. Its SHA256 is
`b86858aa8c495b2a748f76a763729fd5f8b7e6792ebfd3a3c52c89c2ea824195`.
The isolated job is collected, with unchanged inputs and no limit events.
This includes the decoder covered by the 31 accepted diagnostic cases.

The application link failed with nine E0463 crate-resolution errors. Its direct
dependencies were pinned and mounted, but the older build manifest omitted
transitive libraries that had been visible through its broader filesystem view.
The stricter isolated build exposed that preparation gap. Retained Cargo
dependency records identify missing derive and builder dependencies; their
complete binding is being prepared before a fresh link-only attempt.

The failed attempt is collected. Known processes, unit and cgroup are absent,
the shared lock is free, and inputs remain unchanged. No application or SDK
was launched. The accepted worker library will be reused.

The prepared V3 configuration retains 32 direct pins and 1,259 source and
dependency hashes, all checked. The existing native host and 19 support files
also match their retained bindings; all 16 generation inputs match current
source. The older application binary is excluded from the new assembly.

The normal repair candidate is unchanged and remains independently authorized.
Its deployment still requires active calls to finish. The new V3 application
binary, private account binding and real upstream acceptance remain unfinished.
