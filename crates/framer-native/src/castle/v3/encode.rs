use super::{deflate, envelope, serialize, EnvelopeError, Value};

#[derive(Debug, PartialEq, Eq)]
pub enum EncodingError {
    CollectedValueTooLarge,
    Clock,
    Envelope(EnvelopeError),
}

/// Encode the complete supplied SDK value array using its fallback compressor.
///
/// This is the post-collection boundary at SDK bytes 274980..281222. Values
/// retain their positions and types, including undefined entries. The caller
/// owns collection, readiness and any custom JavaScript object conversion.
/// No missing observations are invented and no service acceptance is implied.
pub fn encode_collected(values: &[Value], timestamp_ms: i64) -> Result<String, EncodingError> {
    encode_with_clock(values, || Ok(timestamp_ms))
}

/// Live clock sample follows serialization and compression, as in the SDK.
pub(crate) fn encode_current(values: &[Value]) -> Result<String, EncodingError> {
    encode_with_clock(values, || {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| EncodingError::Clock)?.as_millis().try_into()
            .map_err(|_| EncodingError::Clock)
    })
}

fn encode_with_clock(values: &[Value], clock: impl FnOnce() -> Result<i64, EncodingError>)
    -> Result<String, EncodingError> {
    let compressed =
        deflate(&serialize(values).map_err(|_| EncodingError::CollectedValueTooLarge)?);
    envelope(&compressed, clock()?).map_err(EncodingError::Envelope)
}
