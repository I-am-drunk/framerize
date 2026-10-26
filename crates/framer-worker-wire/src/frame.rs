use crate::{WireError, FRAME_LIMIT};
use serde::{de::DeserializeOwned, Serialize};
use std::io::{self, Write};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

struct Limited(Vec<u8>);
impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > (FRAME_LIMIT + 4).saturating_sub(self.0.len()) {
            return Err(io::Error::other("frame byte limit"));
        }
        self.0.try_reserve_exact(bytes.len())
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, WireError> {
    let mut encoded = Limited(vec![0; 4]);
    if let Err(error) = serde_json::to_writer(&mut encoded, message) {
        return Err(match error.io_error_kind() {
            Some(io::ErrorKind::OutOfMemory) => WireError::Allocation,
            Some(_) => WireError::FrameLimit,
            None => error.into(),
        });
    }
    let length = (encoded.0.len() - 4) as u32;
    encoded.0[..4].copy_from_slice(&length.to_be_bytes());
    Ok(encoded.0.into_boxed_slice().into_vec())
}

pub struct Decoded<T> { pub message: T, pub wire_bytes: u64 }

/// The stream owner must retain this read until completion or channel teardown.
/// Decode directly into strict DTOs, never an intermediate serde_json::Value.
pub async fn read<R: AsyncRead + Unpin, T: DeserializeOwned>(reader: &mut R)
    -> Result<Option<Decoded<T>>, WireError> {
    let mut prefix = [0u8; 4];
    if reader.read(&mut prefix[..1]).await? == 0 { return Ok(None); }
    reader.read_exact(&mut prefix[1..]).await?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length == 0 || length > FRAME_LIMIT { return Err(WireError::FrameLimit); }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| WireError::Allocation)?;
    bytes.resize(length, 0);
    reader.read_exact(&mut bytes).await?;
    let message = serde_json::from_slice(&bytes)?;
    Ok(Some(Decoded { message, wire_bytes: (length + 4) as u64 }))
}

/// A started write is owned through completion. Cancellation of its operation
/// cannot cancel this future; I/O failure instead ends the whole channel.
pub async fn write<W: AsyncWrite + Unpin>(writer: &mut W, frame: &[u8])
    -> Result<(), WireError> {
    if frame.len() < 5 || frame.len() > FRAME_LIMIT + 4
        || u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize != frame.len() - 4 {
        return Err(WireError::FrameLimit);
    }
    writer.write_all(frame).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;
