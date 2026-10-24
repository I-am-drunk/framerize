# Ordinary V2 prepared-host package closure

Source-only input on `fix/sdk-seed-package-closure-20261024`, based exactly on
`3d52cde09bf4c3e9d1a7b934e3ecbacea755767a`. This branch changes the package
generator, startup hook, one new fixture file and this note. It does not
include the separately owned producer or consumer implementation.

The producer contract names `scripts/sdk_seed_observation.py` and the global
module `sdk_seed_observation`. Its eager dependency is
`contextual_authorization.login_cookie`; its resident observer import is lazy.
Structure owns the real helper, host producer and native installation logic.
That implementation injects `Host._sdk_seed` and `Host._sdk_seed_enabled` from
`native_device.install`. No placeholder helper is supplied by this branch.

`prepare_browser_runtime.prepare` now requires and copies the real helper into
the hash-named support directory. It includes its SHA-256 in the existing
manifest calculation alongside all ten previous support sources. The
manifest, directory name and generated host hash continue to derive from the
actual source bytes. No existing package, manifest or saved pin is rewritten.
An absent source helper fails during hashing, before destination creation.

The startup hook retains this order:

1. Verify the manifest and every support source hash.
2. Load `resident_activity`, `contextual_authorization`,
   `resident_authorization`, `resident_bootstrap`, then `sdk_seed_observation`
   by exact support path, registering each global module before executing it.
3. Load the cooldown package and install its existing instance-root handling.
4. Import the package's `native_device` and call its existing `install`.
5. Recheck every support source hash before constructing `Host` and entering
   its loop.

The private runtime-root derivation, cooldown restoration, native rate options
and optional enrollment lifecycle argument are unchanged. This patch does not
change explicit script overrides or `sidecar.rs`. The raw embedded host has
neither this package hook nor native installation. Combined qualification
must use an explicit, freshly prepared host with its exact support directory;
the embedded executable alone cannot establish this closure.

## Fixture selection and limits

All six new selections in
`scripts/tests/test_sdk_seed_package_closure.py::SdkSeedPackageClosure` are
**UNRUN** in this source lane:

- `test_prepared_support_copies_and_pins_exact_sources`
- `test_helper_is_loaded_before_native_install_and_host_loop`
- `test_missing_source_helper_refuses_before_creating_package`
- `test_missing_packaged_helper_refuses_before_loading_support`
- `test_preload_helper_tamper_refuses_before_loading_support`
- `test_postload_helper_tamper_refuses_before_host_construction`

The first selection reads the combined checkout's actual source bytes and
checks the complete copied inventory and generated hashes without importing
the application. It requires Structure's real helper to be present. The
remaining selections copy the actual generator and hook into an isolated
temporary tree containing only synthetic host/support modules. They check
load order and native argument preservation for enrollment off and on,
missing-input refusal, and refusal before host construction after tampering.

Synthetic helper code exists only inside the fixture's temporary source tree.
The fixture blocks fallback imports of support dependencies, restores module
registration and bytecode settings, and never starts a browser, subprocess,
network listener or service. It does not inspect real account or runtime data.
These fixtures prove packaging behavior when run, not actual SDK semantics,
valid minting, accepted inference or account isolation. Post-load validation
detects changed source before host construction; it does not undo import-time
side effects or establish continuous source immutability.

No test, build, application import or package generation ran in this lane.
Only source inspection and Git diff checks are used for this handoff. No
runtime acceptance result or memory claim is added. The qualified T3 package,
help result and notes remain untouched and T3 work remains held.

## Required composition

Structure's real producer input is published and verified at commit
`56284770c1244449383df3d7269e39a69b2ba7a0`, with exact parent
`3d52cde09bf4c3e9d1a7b934e3ecbacea755767a` and tree
`48d4b1690472904b6995bcb6a5247596a03a46fa`.

| Producer source | Git blob |
| --- | --- |
| `scripts/sdk_seed_observation.py` | `8981c086b024ef621b8f54fd05c9bee42c8508ab` |
| `scripts/native_device.py` | `a8bee36d886fdd95a5c5b4fd8fa1f17701183315` |
| `scripts/camoufox_host.py` | `75383de0d582d1900c23fa6d56f8c0a31504380e` |

The real helper's independently checked SHA-256 is
`8bd6a5bd1debc86005e2def2d05a604f6b16a22cbde57531f0d7083ce883b83d`.
Source inspection confirms its eager contextual import and lazy resident
import match the package order. Native installation injects and enables the
helper; the raw host has no unconditional helper import. No application code
was imported to check these facts. The 21 producer cases remain unrun here.

This producer commit remains a separate input, not a merge into the package
branch. Intake must bind it with Transport's consumer and this package patch
before runtime qualification. No existing frozen input may substitute a
synthetic helper or an HTTP-device fallback.

Runtime owner `544711e3-0b76-4cb4-9732-124a0486847e` owns generation of the fresh
explicit host, the six selections above, affected startup controls and any
combined/native acceptance. Bind the resulting host hash, support manifest
hash and every copied source hash to the composed source commit. This source
contribution does not grant execution, installation or deployment.
