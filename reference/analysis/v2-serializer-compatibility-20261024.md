# V2 serializer compatibility, 2026-10-24

The selected Framer V2 factory has SHA-256
`df5a150be914e7d267126f35de92934993e7930f18c3fd00bf79a86bd788ec58`.
Static extraction identifies wire format 13, five fingerprint groups
`0, 4, 7, 8, 9`, and two-byte group headers with an 11-bit group index and
5-bit entry count. The third word at its outer encryption call differs
from Yubie's public decoder; the other three words agree.

The group selector begins at byte 517376, header encoder at 518393,
group serializer at 518905 and token construction at 519214 of that exact
factory file. The version initializer at 356248 resolves to 13. These
are static source facts, not observations from a private token.

[Yubie's decoder at e335f479](https://github.com/yubie-re/castleio-gen/blob/e335f47901bfbb8468c83cdf9d7150b5505b8c92/castleio/DecodeToken.py)
expects three groups and one-byte group headers. Its README's V11/SDK 2.6.0
limit does not rule out updating it. The concrete differences above define
part of that work; they do not prove that changing only those fields is
sufficient. The original entry point prints decoded identities and fields,
so it must not be used directly for a private structural comparison.

[castle-rs at 15421663](https://github.com/0x640480/castle-rs/blob/1542166339899321f81fdb4f128274b5c7359173/tests/decode_roundtrip.rs#L72-L89)
also names groups `0, 4, 7, 8, 9`. Its local round trip is a useful source
cross-reference, not independent Framer acceptance evidence.

The local source proof is `yubie-decoder-map-r1/serializer-proof.json`,
SHA-256 `2277a92101204ed781e0337c9e4af48947b85103028a63ace6c522ad5a3328c4`.
REA parsed the two exact source slices without execution under Evidence
`ev_69927634ebe9e649c8ac85166bff9e197061694818cdf804ae595089b369bd88`.
The obfuscated semantic graph remains partial. Numerical conclusions come
from the separately bound constant derivation. No generator, private token
decoder or upstream request ran for this comparison.

Next: establish the remaining current field framing and implement a bounded
structural decoder with synthetic vectors before comparing private inputs.
