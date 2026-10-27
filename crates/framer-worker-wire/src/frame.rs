use crate::{WireError, FRAME_LIMIT};
use serde::{de::DeserializeOwned, Serialize};
use std::io::{self, Write};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use std::{collections::{BTreeMap, VecDeque}, sync::{Arc, Mutex}};
use tokio::sync::{oneshot, Notify, OwnedMutexGuard};

struct WriteRequest {
    bytes: Vec<u8>,
    reply: oneshot::Sender<Result<(), ()>>,
    _permit: OwnedMutexGuard<()>,
}
struct WriteRecord {
    gate: Arc<tokio::sync::Mutex<()>>,
    pending: Option<WriteRequest>,
}
struct QueueState {
    records: BTreeMap<u64, WriteRecord>,
    ready: VecDeque<u64>,
    closed: bool,
}
struct QueueInner { state: Mutex<QueueState>, wake: Notify, limit: usize }
#[derive(Clone)]
pub struct Outbox(Arc<QueueInner>);
/// One owning consumer. Dropping it closes every producer.
pub struct FrameWriter(Outbox);
#[derive(Clone)]
pub struct WriteSlot { outbox: Outbox, op: u64, gate: Arc<tokio::sync::Mutex<()>> }
/// Reserve before allocating a frame. At most one allocation per slot exists.
pub struct WritePermit { slot: WriteSlot, permit: OwnedMutexGuard<()> }
/// Receipt for a frame already owned by the writer. Dropping this receipt
/// cannot remove the frame or cancel its write.
pub struct WriteReceipt(oneshot::Receiver<Result<(), ()>>);
impl WriteReceipt {
    pub async fn wait(self) -> Result<(), WireError> {
        self.0.await.map_err(|_| WireError::Protocol)?.map_err(|_| WireError::Protocol)
    }
}

impl Outbox {
    pub fn new(limit: usize) -> Result<(Self, FrameWriter), WireError> {
        if limit == 0 { return Err(WireError::Protocol); }
        let outbox = Self(Arc::new(QueueInner {
            state: Mutex::new(QueueState {
                records: BTreeMap::new(), ready: VecDeque::new(), closed: false,
            }), wake: Notify::new(), limit,
        }));
        Ok((outbox.clone(), FrameWriter(outbox)))
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, QueueState> {
        self.0.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl Outbox {
    /// The caller owns the fixed record limit and its operation/lane mapping.
    pub fn register(&self, op: u64) -> Result<WriteSlot, WireError> {
        let mut state = self.lock();
        if state.closed || state.records.len() == self.0.limit
            || state.records.contains_key(&op) { return Err(WireError::Protocol); }
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        state.records.insert(op, WriteRecord { gate: gate.clone(), pending: None });
        Ok(WriteSlot { outbox: self.clone(), op, gate })
    }
    /// Remove only a settled slot. A stale clone cannot address a reused key.
    pub fn remove(&self, slot: &WriteSlot) -> Result<(), WireError> {
        if !Arc::ptr_eq(&self.0, &slot.outbox.0) { return Err(WireError::Protocol); }
        let _permit = slot.gate.try_lock().map_err(|_| WireError::Protocol)?;
        let mut state = self.lock();
        let record = state.records.get(&slot.op).ok_or(WireError::Protocol)?;
        if !Arc::ptr_eq(&record.gate, &slot.gate) || record.pending.is_some() {
            return Err(WireError::Protocol);
        }
        state.records.remove(&slot.op);
        Ok(())
    }
}

impl Outbox {
    /// Stop accepting frames; the writer finishes every accepted frame.
    pub fn close(&self) {
        self.lock().closed = true;
        self.0.wake.notify_one();
    }
    /// Channel teardown only. Pending writes fail; no successful credit returns.
    pub fn poison(&self) {
        let mut state = self.lock();
        state.closed = true;
        state.ready.clear();
        for record in state.records.values_mut() { record.pending.take(); }
        drop(state);
        self.0.wake.notify_one();
    }
}
impl WriteSlot {
    pub async fn reserve(&self) -> Result<WritePermit, WireError> {
        let permit = self.gate.clone().lock_owned().await;
        let state = self.outbox.lock();
        let record = state.records.get(&self.op).ok_or(WireError::Protocol)?;
        if state.closed || !Arc::ptr_eq(&record.gate, &self.gate) {
            return Err(WireError::Protocol);
        }
        Ok(WritePermit { slot: self.clone(), permit })
    }
}

impl WriteSlot {
    pub async fn send<T: Serialize>(&self, message: &T) -> Result<(), WireError> {
        self.reserve().await?.send_encoded(encode(message)?).await
    }
}
impl WritePermit {
    pub async fn send_encoded(self, bytes: Vec<u8>) -> Result<(), WireError> {
        self.enqueue_encoded(bytes)?.wait().await
    }
    /// Synchronous acceptance lets a caller publish its local sequence record
    /// under the same lock that its reply dispatcher consults.
    pub fn enqueue_encoded(self, bytes: Vec<u8>) -> Result<WriteReceipt, WireError> {
        validate_frame(&bytes)?;
        if bytes.capacity() > FRAME_LIMIT + 4 { return Err(WireError::FrameLimit); }
        let (reply, receive) = oneshot::channel();
        {
            let mut state = self.slot.outbox.lock();
            if state.closed { return Err(WireError::Protocol); }
            let record = state.records.get_mut(&self.slot.op).ok_or(WireError::Protocol)?;
            if !Arc::ptr_eq(&record.gate, &self.slot.gate) || record.pending.is_some() {
                return Err(WireError::Protocol);
            }
            record.pending = Some(WriteRequest { bytes, reply, _permit: self.permit });
            state.ready.push_back(self.slot.op);
        }
        self.slot.outbox.0.wake.notify_one();
        Ok(WriteReceipt(receive))
    }
}

impl FrameWriter {
    pub async fn run<W: AsyncWrite + Unpin>(self, mut writer: W) -> Result<(), WireError> {
        loop {
            let wake = self.0.0.wake.notified();
            let next = {
                let mut state = self.0.lock();
                match state.ready.pop_front() {
                    Some(op) => state.records.get_mut(&op).and_then(|r| r.pending.take()),
                    None if state.closed => return Ok(()),
                    None => None,
                }
            };
            if let Some(request) = next {
                let WriteRequest { bytes, reply, _permit } = request;
                let result = write(&mut writer, &bytes).await;
                drop(bytes);
                drop(_permit);
                let _ = reply.send(result.as_ref().map(|_| ()).map_err(|_| ()));
                result?;
            } else {
                wake.await;
            }
        }
    }
}
impl Drop for FrameWriter {
    fn drop(&mut self) { self.0.poison(); }
}

struct Limited { bytes: Vec<u8>, allocation_limit: usize }
impl Write for Limited {
    #[inline]
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.allocation_limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("frame byte limit"));
        }
        let needed = self.bytes.len() + bytes.len();
        if needed > self.bytes.capacity() {
            let capacity = needed.max(self.bytes.capacity().saturating_mul(2))
                .min(self.allocation_limit);
            self.bytes.try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, WireError> {
    encode_with_limit(message, FRAME_LIMIT + 4)
}

/// Bound the encoder allocation, including its four-byte length prefix.
/// Callers with byte credit must pass their already-reserved capacity.
pub fn encode_with_limit<T: Serialize>(message: &T, allocation_limit: usize)
    -> Result<Vec<u8>, WireError> {
    if !(5..=FRAME_LIMIT + 4).contains(&allocation_limit) {
        return Err(WireError::FrameLimit);
    }
    let mut encoded = Limited { bytes: vec![0; 4], allocation_limit };
    if let Err(error) = serde_json::to_writer(&mut encoded, message) {
        return Err(match error.io_error_kind() {
            Some(io::ErrorKind::OutOfMemory) => WireError::Allocation,
            Some(_) => WireError::FrameLimit,
            None => error.into(),
        });
    }
    let length = (encoded.bytes.len() - 4) as u32;
    encoded.bytes[..4].copy_from_slice(&length.to_be_bytes());
    Ok(encoded.bytes.into_boxed_slice().into_vec())
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
    validate_frame(frame)?;
    writer.write_all(frame).await?;
    writer.flush().await?;
    Ok(())
}

fn validate_frame(frame: &[u8]) -> Result<(), WireError> {
    if frame.len() < 5 || frame.len() > FRAME_LIMIT + 4
        || u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize != frame.len() - 4 {
        return Err(WireError::FrameLimit);
    }
    Ok(())
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;
