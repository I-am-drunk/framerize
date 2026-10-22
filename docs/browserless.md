# Experimental browserless path

This branch preserves the accepted native routing source separately from the
browser-based `main` and `browser-based` branches. It is unfinished and has
not delivered a real inference response through the complete new path.

The optional `native-dispatch` feature compiles the native path. The executable
only configures it when given explicit offline worker and routing settings.
Normal browser serving remains available. Do not replace a live service or
run a second writer against its accounts merely to try this branch.

The source already connects request context, shared session ownership, native
transport, streaming and cleanup. The remaining immediate connection is from
the restored shared device/rate authority to the worker's private version-2
input and its joined cleanup. Follow `development-status.md` for evidence and
the exact next step.

Preserve one authority for account cooldowns. Use one bounded real request
after that connection is validated; do not mirror paid traffic. Component
tests and a successful standalone SDK mint do not prove client delivery or
the whole service's memory use.

The source-only device components are now tracked in `scripts/native_device.py`
and `scripts/offline_worker/`. Their fixtures are under
`scripts/tests/native_device/` and beside the worker modules. Their 23 and 28
cases now pass in their original bounded runs. These files are not installed
or automatically connected to the live host; application integration remains
in progress.

The worker's pin table still names the original local, verified SDK and JSDOM
inputs. Those external inputs are not bundled in Git. Keep this limitation
explicit until the runtime package is made relocatable and validated. Never
substitute unverified dependencies just to make a copied worker launch.
