//! Encoding primitives from the pinned Framer V3 SDK asset.
//!
//! V3 names Framer's selected path, not a Castle package or token-format version.

mod value;
mod json;
pub use value::{serialize, Value};
mod deflate;
pub use deflate::deflate;
mod envelope;
pub use envelope::{envelope, EnvelopeError};
mod worker_result;
pub use worker_result::{shared_worker_version, NormalizedWorkerValue};
mod network_result;
pub use network_result::{network_address, NetworkValueError};

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

/// Append a signed integer using the SDK's zigzag transform and unsigned writer.
/// The original expression is at bytes 275909..275928 of the pinned asset.
pub fn write_signed(value: i32, output: &mut Vec<u8>) {
    write_unsigned(((value as u32) << 1) ^ ((value >> 31) as u32), output);
}

/// Append the SDK's fixed-width, little-endian unsigned integer representation.
/// The original writer is at bytes 275842..275897 of the pinned asset.
pub fn write_fixed_unsigned(value: u32, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_le_bytes());
}

/// Append UTF-16 code units prefixed by their count, as the SDK's `S` does.
/// The original writer is at bytes 276160..276254 of the pinned asset.
///
/// Raw code units preserve JavaScript strings containing unpaired surrogates.
/// Inputs longer than `u32::MAX` units are refused before changing the output.
/// String coercion and the enclosing value's type tag belong to the caller.
pub fn write_utf16(
    value: &[u16],
    output: &mut Vec<u8>,
) -> Result<(), std::num::TryFromIntError> {
    let length = u32::try_from(value.len())?;
    write_unsigned(length, output);
    for unit in value {
        output.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}
