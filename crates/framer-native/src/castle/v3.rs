//! Encoding primitives from the pinned Framer V3 SDK asset.
//!
//! V3 names Framer's selected path, not a Castle package or token-format version.

/// Append the SDK's unsigned base-128 representation without shared state.
///
/// Matches `R` and `G` in SDK SHA256
/// `3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71`,
/// byte ranges 274997..275051 and 276131..276160.
/// The caller supplies a `u32`; JavaScript's preceding `ToUint32` coercion is
/// outside this interface. At most five bytes are appended to `output`.
pub fn write_unsigned(mut value: u32, output: &mut Vec<u8>) {
    while value >= 128 {
        output.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    output.push(value as u8);
}
