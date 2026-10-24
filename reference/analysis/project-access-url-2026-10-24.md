# Project URL access suffix

The public project URL builder permits an access-token suffix after the
project ID. Our readiness, Python page check and Rust credential target
required the project ID to end the path, so they rejected that valid form.

Current `chunk-YTS3KU4H.mjs`, export `b`, builds a sanitized title followed by
`--`, the project ID, and optional `-accessToken`. Its function occupies
ASCII offsets 303 through 450. SHA256:
`1c35a5597bb1025ccf9c288d74d7546c7afd093ccd1f13eb8885dfa980aadabd`.
REA source analysis: `ev_e349e851541228bde37c7f90df6afcbc653547f14bcd65ef4bc55c25ce1bb636`.

The retained public bootstrap parser independently recognizes a 20-character
alphanumeric project ID and optional alphanumeric access suffix. Bootstrap
SHA256: `55726fa398c002ea23d5b171b26dc7ad99a0ee81d92f8cddfcdf62152eac9727`.
This suffix is project access data, not a Castle or inference bearer token.

The correction checks the entire additional path form and compares its
parsed project ID exactly. It retains origin, nested-path and size checks.
The exact full URL is still bound afterward; later URL changes still refuse.
No access token is recorded in diagnostics or fixtures.

R15's diagnostic reported `project_match=false` with all seven history/device
checks true. That narrows the failure but does not establish this suffix as
its cause, because the actual URL was not retained. Python and exact-JS
fixtures pass; Rust and live validation are recorded in development status.
