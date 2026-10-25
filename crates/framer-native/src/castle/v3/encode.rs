use super::{deflate, envelope, serialize, EnvelopeError, Value};

#[derive(Debug, PartialEq, Eq)]
pub enum EncodingError {
    CollectedValueTooLarge,
    Envelope(EnvelopeError),
}

/// Encode the complete supplied SDK value array using its fallback compressor.
///
/// This is the post-collection boundary at SDK bytes 274980..281222. Values
/// retain their positions and types, including undefined entries. The caller
/// owns collection, readiness and any custom JavaScript object conversion.
/// No missing observations are invented and no service acceptance is implied.
pub fn encode_collected(values: &[Value], timestamp_ms: i64) -> Result<String, EncodingError> {
    let compressed =
        deflate(&serialize(values).map_err(|_| EncodingError::CollectedValueTooLarge)?);
    envelope(&compressed, timestamp_ms).map_err(EncodingError::Envelope)
}
