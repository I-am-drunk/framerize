# Native network signature ordering

`collection::NetworkCollator` provides native locale ordering for the existing
network record assembler. It uses the locked ICU4X 2.3 collation implementation.
The initial ICU4X 2.0 candidate matched the selected corpus but conflicted
with the existing URL/IDNA dependency requirements. No dependency was downgraded.

The caller supplies the observation runtime's locale. There is no service-host
default. The supported profiles are en-US, en-GB, de, fr, fr-CA, sv, da, tr,
pl, cs, ja, zh, th, ar, es-u-co-trad, de-u-co-phonebk, en-US-u-kn-true and
en-US-u-kf-upper. These cover default sort settings plus explicit numeric,
case-order and collation variants. Unknown profiles are refused.

Lone UTF-16 surrogates are refused before comparing or assembling selected
candidates. The original JavaScript engine distinguishes those code units;
ICU4X treats them as replacement characters. This is a known incompatibility,
not a reason to silently change the input. Valid surrogate pairs are supported.
Unsorted history strings retain their original code units. Candidate validation
stops at the first 16 entries, matching the assembler's existing selection cap.

`python3 -I -B scripts/check_native_collation.py` compiles only the relevant
Rust modules and the workspace's exact locked dependency versions. It runs
four boundary tests, compares all pairs of 86 synthetic strings across 18
profiles, and compares 30 complete records with the pinned SDK's original
finalizer. A further selected malformed record is explicitly refused.
The SDK module is not loaded and no collection, account or network call runs.

All checks pass with Node 20.20.0 and the CI runtime Node 24.13.0, both using
ICU 77.1. Each run compares 133,128 ordering pairs and verifies 35 malformed
comparisons are refused. All four boundary tests pass, including a single
malformed candidate that would not trigger any sort comparisons.

An earlier harness selected system Node 18.19.1 by mistake. That runtime
ordered empty and ignorable strings differently. The corrected harness binds
the selected Node explicitly. Locale alone is therefore insufficient to
claim compatibility across arbitrary engines or runtime versions.

This finite corpus is evidence for the selected runtime profiles, not a proof
for every Unicode string. The original supplied-comparator API remains usable
for other profiles. Genuine collection, observation ownership, serving
integration, live acceptance and request-time RAM savings are still unverified.
