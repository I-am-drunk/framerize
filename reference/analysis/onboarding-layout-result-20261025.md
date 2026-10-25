# Dedicated onboarding layout preparation

The fourteen selected layout and integration checks passed with no skips.
The first collected kernel check then failed before executing its probe:
Bubblewrap 0.9.0 refused more than 9,000 arguments. The file-by-file mount
list fits Linux's byte limit, but exceeds Bubblewrap's separate count limit.
Upstream Bubblewrap also counts arguments supplied through `--args`, so
moving this list into an argument file would not correct the failure.

The failed check is collected. Its four source pins are unchanged, its
known processes and unit are absent, and the shared execution lock is free.
Collection: `3750221fd9f649c340c0baf850e06dd6f67341e2fbb4f9a3a2fca9f8fce9e506`.
The existing onboarding invocation is not qualified for launch.

A separate sparse-tree copier now has six passing focused checks. It verifies
source bytes, size, mode and identity, creates independent files, refuses
replacement and symlink traversal, and verifies the copied bytes. These checks
used the existing isolated 128 MiB harness with zero skips. Collection:
`e928fd50fea04613b2c00efad0ee9c9247a86e25d6bf1b3767cd1f165a070e28`.

The bounded copy command completed successfully in six batches: 6,065 files
and 2,211,163,850 bytes. Each batch exposed only its selected source files to
the copier. The command retained the shared execution lock, memory, task,
CPU, swap and deadline limits. No copied executable was started.
Final verification matched every file's bytes, size and mode. The known
processes and unit are absent, and the execution lock is free. Collection:
`2e7a3298ea6883dfa071736dd913c9181a4d2a65bf04656ef5f96901c0f9706b`.

The compact mount plan remains a source-only assignment. It must retain all
selected files and aliases, writable generated caches, and the empty account
store while exposing only the staged sparse tree. Its integration still needs
a successful kernel-layout check before application startup.

The adjacent patches preserve the selected layout and copier source.
No account login, SDK operation, service deployment or native acceptance
is established by these results. The independent repair rollout is unchanged.
