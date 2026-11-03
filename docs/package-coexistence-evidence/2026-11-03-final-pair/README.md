# Final retained package-pair verification

On 2026-11-03, the independent reviewer ran the root coexistence checker against
the existing qualified Nexus tarball and final reviewed server tarball. Both
Linux x64 runs passed, using Node 22.23.4 with npm 10.9.9 and Node 24.13.0 with
npm 11.6.2. No tarball was rebuilt or downloaded for this check.

## Exact inputs

- `framerize-nexus@0.1.9`, source
  `671ed7df4507b391fc6efc72bf919aebd289a1eb`.
  Tarball SHA-256:
  `12c6eb43755f16a858d12b150bfde44622298ab2fcf489d03abadba0bd4fe528`.
- `framerize-server@0.1.0`, source
  `23d87c6c9a04952514eed9eebc6936e61b6f3e05`.
  Tarball SHA-256:
  `c4282b29d43ee8fa38856cd26a4778db82926548fb329d753e5306924d2c2ff3`.

The checker is `scripts/package-coexistence.mjs` from reviewed commit
`0488bb6df8a3e19ae9c36f4e1537569c9265cb9b`, unchanged at `179b99de`.
Its SHA-256 is
`444bcfbed177f317961f6403d34b67883a2eda9de43bcb82643e6fdf34f22274`.

Source identities come from prior independent archive-to-Git comparisons,
not from tarball filenames or the checker's output. The Nexus artifact came
from evidence-only run `41654672886`; the server artifact came from qualified
run `41663296507`. Their native and aggregate checks remain separate evidence.

## Retained reports

These files preserve the reviewer's original JSON bytes:

- [Node 22.23.4 report](node-22.23.4.json), SHA-256
  `ae019087a2dfd9466f092ab29ce4638f2f034e4320d71289a5b53af752c80424`.
- [Node 24.13.0 report](node-24.13.0.json), SHA-256
  `fda387391168487f0b820d7c7dcfba2908514752101377e3b1aa8491c665de88`.

Both runs installed the exact two tarballs together, offline, in a private
temporary directory. The checker compared all 13 installed files per package
with the tarballs, allowing only npm's documented launcher shebang change.
It checked distinct executable targets, direct and offline npm CLI help,
CLI configuration checks, and the two Nexus and five baseline server exports.
Network and listener guards were enabled. The reviewer confirmed that each
private temporary directory was empty after the check.

## Limits and next gate

This is Linux package-install evidence for these exact inputs. It does not
qualify macOS or Windows coexistence, an actual merged source tree, later
package changes, the separate Castle library, live requests, publication,
or service activation. The reports contain no binaries or credentials.

The integration owner retains the merge decision and approval of a separate
three-OS coexistence workflow. That workflow must test the same pair of
tarballs on every OS and retain their hashes. Existing Nexus and server native
CI does not run this root checker and cannot establish that result.
