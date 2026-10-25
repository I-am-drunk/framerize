//! Bounded SSE framing, with one decoder per stream. No HTTP, JSON conversion,
//! tasks, global state, retries, or event queue. Payloads remain opaque.
//! EOF discards incomplete events. Callers must require a terminal event.

use std::fmt;
use std::ops::ControlFlow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum raw and decoded UTF-8 bytes in a line, excluding its ending.
    pub line_bytes: usize,
    /// Maximum live decoded data + event name + last-event-id bytes.
    pub event_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    InvalidLimits, LineLimit, EventLimit, AllocationFailed, Cancelled, Finished,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidLimits => "sse: invalid byte limits",
            Self::LineLimit => "sse: line exceeds byte limit",
            Self::EventLimit => "sse: event exceeds byte limit",
            Self::AllocationFailed => "sse: allocation failed",
            Self::Cancelled => "sse: decoder cancelled",
            Self::Finished => "sse: decoder finished",
        })
    }
}
impl std::error::Error for DecodeError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event<'a> {
    pub event: &'a str,
    pub data: &'a str,
    pub last_event_id: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State { Open, Finished, Failed(DecodeError) }

pub struct Decoder {
    limits: Limits,
    state: State,
    line: Vec<u8>,
    data: String,
    event: String,
    last_event_id: String,
    first_line: bool,
    skip_lf: bool,
    has_data: bool,
}

impl Decoder {
    pub fn new(limits: Limits) -> Result<Self, DecodeError> {
        if limits.line_bytes == 0 || limits.event_bytes == 0
            || limits.line_bytes > limits.event_bytes
        {
            return Err(DecodeError::InvalidLimits);
        }
        Ok(Self {
            limits, state: State::Open, line: Vec::new(), data: String::new(),
            event: String::new(), last_event_id: String::new(),
            first_line: true, skip_lf: false, has_data: false,
        })
    }

    /// Each event borrows storage until this synchronous callback returns.
    /// Break cancels before later bytes are consumed. Errors clear buffers.
    /// Async owners must feed bounded input only when downstream is ready.
    pub fn feed<F>(&mut self, input: &[u8], mut on_event: F) -> Result<(), DecodeError>
    where F: FnMut(Event<'_>) -> ControlFlow<()>,
    {
        match self.state {
            State::Open => {}
            State::Finished => return Err(DecodeError::Finished),
            State::Failed(error) => return Err(error),
        }
        let result = self.feed_inner(input, &mut on_event);
        if let Err(error) = result {
            self.state = State::Failed(error);
            self.clear_owned();
        }
        result
    }

    fn feed_inner<F>(&mut self, input: &[u8], on_event: &mut F) -> Result<(), DecodeError>
    where F: FnMut(Event<'_>) -> ControlFlow<()>,
    {
        let mut offset = 0;
        while offset < input.len() {
            if self.skip_lf {
                self.skip_lf = false;
                if input[offset] == b'\n' {
                    offset += 1;
                    continue;
                }
            }
            let rest = &input[offset..];
            let run = rest.iter().position(|byte| matches!(*byte, b'\r' | b'\n'))
                .unwrap_or(rest.len());
            self.append_line(&rest[..run])?;
            offset += run;
            if offset == input.len() { break; }
            self.skip_lf = input[offset] == b'\r';
            offset += 1;
            self.process_line(on_event)?;
        }
        Ok(())
    }

    fn append_line(&mut self, bytes: &[u8]) -> Result<(), DecodeError> {
        let needed = self.line.len().checked_add(bytes.len())
            .ok_or(DecodeError::LineLimit)?;
        if needed > self.limits.line_bytes { return Err(DecodeError::LineLimit); }
        if needed > self.line.capacity() {
            let target = needed.max(self.line.capacity().saturating_mul(2))
                .min(self.limits.line_bytes);
            self.line.try_reserve_exact(target - self.line.len())
                .map_err(|_| DecodeError::AllocationFailed)?;
        }
        self.line.extend_from_slice(bytes);
        Ok(())
    }

    fn process_line<F>(&mut self, on_event: &mut F) -> Result<(), DecodeError>
    where F: FnMut(Event<'_>) -> ControlFlow<()>,
    {
        // Detach only this bounded line so other decoder fields can be updated.
        let mut raw = std::mem::take(&mut self.line);
        let result = {
            let bytes = if self.first_line {
                self.first_line = false;
                raw.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&raw)
            } else { &raw };
            // Line endings cannot split a valid UTF-8 sequence. Network chunks
            // may split one; append_line preserves those bytes until EOL.
            // Invalid UTF-8 has standard replacement semantics. Temporary
            // decoded storage is at most three times the bounded raw line.
            let line = String::from_utf8_lossy(bytes);
            if line.len() > self.limits.line_bytes {
                Err(DecodeError::LineLimit)
            } else {
                self.process_text_line(&line, on_event)
            }
        };
        raw.clear();
        self.line = raw;
        result
    }

    fn process_text_line<F>(&mut self, line: &str, on_event: &mut F) -> Result<(), DecodeError>
    where F: FnMut(Event<'_>) -> ControlFlow<()>,
    {
        if line.is_empty() {
            let action = if self.has_data {
                on_event(Event {
                    event: if self.event.is_empty() { "message" } else { &self.event },
                    data: &self.data,
                    last_event_id: &self.last_event_id,
                })
            } else { ControlFlow::Continue(()) };
            self.data.clear();
            self.event.clear();
            self.has_data = false;
            return match action {
                ControlFlow::Continue(()) => Ok(()),
                ControlFlow::Break(()) => Err(DecodeError::Cancelled),
            };
        }
        if line.starts_with(':') { return Ok(()); }
        let (name, value) = line.split_once(':').unwrap_or((line, ""));
        // The SSE grammar removes ONE literal space, not general whitespace.
        let value = value.strip_prefix(' ').unwrap_or(value);
        match name {
            "data" => {
                let extra = usize::from(self.has_data);
                let needed = self.data.len().checked_add(extra)
                    .and_then(|n| n.checked_add(value.len()))
                    .ok_or(DecodeError::EventLimit)?;
                self.check_event_size(needed, self.event.len(), self.last_event_id.len())?;
                reserve_text(&mut self.data, needed, self.limits.event_bytes)?;
                if self.has_data { self.data.push('\n'); }
                self.data.push_str(value);
                self.has_data = true;
            }
            "event" => {
                self.check_event_size(self.data.len(), value.len(), self.last_event_id.len())?;
                replace_text(&mut self.event, value, self.limits.line_bytes)?;
            }
            "id" if !value.contains('\0') => {
                self.check_event_size(self.data.len(), self.event.len(), value.len())?;
                replace_text(&mut self.last_event_id, value, self.limits.line_bytes)?;
            }
            // retry is deliberately ignored: this parser never reconnects.
            _ => {}
        }
        Ok(())
    }

    fn check_event_size(&self, data: usize, event: usize, id: usize) -> Result<(), DecodeError> {
        match data.checked_add(event).and_then(|n| n.checked_add(id)) {
            Some(n) if n <= self.limits.event_bytes => Ok(()),
            _ => Err(DecodeError::EventLimit),
        }
    }

    /// EOF discards, rather than dispatches, any unfinished line or event.
    pub fn finish(&mut self) {
        if self.state == State::Open { self.state = State::Finished; }
        self.clear_owned();
    }

    /// Releases decoder-owned state immediately. Does not own an HTTP body.
    pub fn cancel(&mut self) {
        if self.state == State::Open { self.state = State::Failed(DecodeError::Cancelled); }
        self.clear_owned();
    }

    fn clear_owned(&mut self) {
        self.line = Vec::new();
        self.data = String::new();
        self.event = String::new();
        self.last_event_id = String::new();
        self.has_data = false;
        self.skip_lf = false;
    }

    /// Logical retained bytes only, not allocator capacity or process RSS.
    pub fn buffered_bytes(&self) -> usize {
        self.line.len() + self.data.len() + self.event.len() + self.last_event_id.len()
    }
}

fn reserve_text(text: &mut String, needed: usize, limit: usize) -> Result<(), DecodeError> {
    if needed > limit { return Err(DecodeError::EventLimit); }
    if needed > text.capacity() {
        let target = needed.max(text.capacity().saturating_mul(2)).min(limit);
        text.try_reserve_exact(target - text.len())
            .map_err(|_| DecodeError::AllocationFailed)?;
    }
    Ok(())
}

fn replace_text(text: &mut String, value: &str, limit: usize) -> Result<(), DecodeError> {
    text.clear();
    reserve_text(text, value.len(), limit)?;
    text.push_str(value);
    Ok(())
}

#[cfg(test)]
#[path = "sse_decoder_tests.rs"]
mod tests;
