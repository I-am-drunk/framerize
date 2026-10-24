# Native SDK envelope inspection

This experimental inspector checks outer framing for the pinned Castle SDK
`3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71`.
It returns lengths only and clears its decoded byte buffer. It does not
authenticate tokens, decrypt features or establish Framer acceptance.

The source packs a two-byte little-endian auxiliary length, the auxiliary
bytes, and a protected payload with at least 24 bytes of framing. The auxiliary
value comes from `Date.now().toString()`, encoded as two bytes per character
and then base64. A current 13-digit timestamp occupies 36 auxiliary bytes.
The source's 65,535-byte overflow branch concerns this clock section.

Four focused cases pass: a complete synthetic frame, the SDK's prefix-only
result, truncated sections, and invalid/oversized encoding. One separate
synthetic SDK run used the previously qualified worker and owner with only
the attached probe added before the private token handoff. It reported
3,951 decoded bytes, 36 clock bytes and 3,913 payload bytes.

The token was discarded after the existing private-pipe handoff. The worker
joined, its process group disappeared, and no swap or memory events occurred.
Group peak was 86,765,568 bytes. No account, service or network request was used.
This is a local structural result, not evidence about the live rejected token.
Production worker code and its deployed bundle are unchanged.

See `qualification.json` for source/result hashes and `worker-probe.patch`
for the exact experimental insertion.
