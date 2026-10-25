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
The original oversized onboarding invocation is not qualified for launch.

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

The compact planner passed eleven selected tests, and tree finalization passed
four focused tests. The completed tree retains all 6,065 files, 749 descendant
directories and 237 literal aliases. Its software mount fragment has 214
arguments. Kernel r3 cleared the argument limit but refused the probe location
under the read-only root. Kernel r4 moved the probe into sandbox temporary
storage and passed without changing the software view.

The kernel verified every mounted file's hash and read-only status, every
alias, both writable generated fontconfig directories, the empty fixture
store, absent production state, and separate PID/network namespaces.
Collection: `92827fe9a9c55aba873fa7277b112110d4da34cab7a5065df641f4365319d84e`.

The r2 runtime adapter passed four exact selected methods with no skips.
Collection: `7c77e381de853484841fff031837e4e546ed99519f58ab9fbfc9bab31ff2311f`.
It refuses a missing or populated r2 directory even if the old r1 is empty.
The final onboarding contract binds 6,096 inputs and retains the accepted
resource, cleanup and account-state boundaries. Unlike the kernel fixture,
the actual onboarding instance shares networking for its local UI and login.

The separate instance started and reached its empty-account readiness check.
The local UI, assets and account endpoint returned HTTP 200. The Add account
form was displayed without submitting a login or inference. The user deferred
sign-in. The owned instance then completed its park and daemon cleanup before
the repair rollout, with zero accounts and zero resident browsers. All 6,096
input hashes matched before and after. The known processes and transient unit
are absent, and the execution lock was released. Collection:
`8f99f51bbba30af362a2998b9d24b18ddc80895658f2b7b84b158346d8bd213e`.

The adjacent patches preserve the selected layout and copier source.
No account login, SDK operation, production service deployment or native acceptance
is established by these results. The independent repair rollout is unchanged.
