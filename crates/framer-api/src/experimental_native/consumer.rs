//! Dormant consumer of the real NativeByteStream. No production export.
//! One owned reducer validates its own dialect's terminal state.
use super::native_http::NativeResponse;
use super::sse_decoder::{DecodeError, Decoder, Event, Limits};
use futures::StreamExt;
use std::ops::ControlFlow;

#[derive(Debug, PartialEq, Eq)]
pub struct ResponseHead {
    pub status: u16,
    pub usage_header: Option<String>,
    pub content_type: Option<String>,
}

pub struct PreparedSse {
    response: NativeResponse,
    decoder: Decoder,
}

pub enum PreparationError {
    NotSuccessfulSse(NativeResponse),
    InvalidLimits { response: NativeResponse, error: DecodeError },
}

/// Rejection returns the ORIGINAL, unpolled response, including its body.
pub fn prepare_native_sse(response: NativeResponse, limits: Limits)
    -> Result<PreparedSse, PreparationError>
{
    if !(200..300).contains(&response.status)
        || !response.content_type.as_deref().is_some_and(is_sse_media_type)
    {
        return Err(PreparationError::NotSuccessfulSse(response));
    }
    match Decoder::new(limits) {
        Ok(decoder) => Ok(PreparedSse { response, decoder }),
        Err(error) => Err(PreparationError::InvalidLimits { response, error }),
    }
}

/// Framing EOF alone is never a result. Inference reducers must make finish()
/// reject missing or failed terminal state. event() must not queue unbounded data.
pub trait Reducer {
    type Output;
    type Error;
    fn event(&mut self, event: Event<'_>) -> Result<(), Self::Error>;
    fn finish(self) -> Result<Self::Output, Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub struct Consumed<T> {
    pub head: ResponseHead,
    pub events: u64,
    pub output: T,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReadError<E> {
    Transport(String),
    Framing(DecodeError),
    Reducer(E),
}

#[derive(Debug, PartialEq, Eq)]
pub struct ReadFailure<E> {
    pub head: ResponseHead,
    pub events: u64,
    pub error: ReadError<E>,
}

impl PreparedSse {
    /// Lazy until polled; no task, queue or retry. Dropping even an unpolled
    /// consume future drops the real body and its account capacity permit.
    /// Success follows transport EOF and reducer.finish(), not a terminal
    /// event alone. Later failed or duplicate events remain observable. The
    /// transport/caller owns deadlines; early successful stop is not offered.
    pub async fn consume<R: Reducer>(self, mut reducer: R)
        -> Result<Consumed<R::Output>, ReadFailure<R::Error>>
    {
        let Self { response, mut decoder } = self;
        let NativeResponse { status, usage_header, content_type, mut body } = response;
        let head = ResponseHead { status, usage_header, content_type };
        let mut events = 0_u64;
        let mut chunks_this_poll = 0;
        while let Some(chunk) = body.next().await {
            let bytes = match chunk {
                Ok(bytes) => bytes,
                Err(error) => return Err(ReadFailure {
                    head, events, error: ReadError::Transport(error),
                }),
            };
            let mut reducer_error = None;
            let decoded = decoder.feed(&bytes, |event| {
                events += 1;
                match reducer.event(event) {
                    Ok(()) => ControlFlow::Continue(()),
                    Err(error) => {
                        reducer_error = Some(error);
                        ControlFlow::Break(())
                    }
                }
            });
            if let Some(error) = reducer_error {
                return Err(ReadFailure { head, events, error: ReadError::Reducer(error) });
            }
            if let Err(error) = decoded {
                return Err(ReadFailure { head, events, error: ReadError::Framing(error) });
            }
            chunks_this_poll += 1;
            if chunks_this_poll == 32 {
                // Even a perpetually ready body cannot monopolize the executor.
                // This retains no extra input and gives cancellation an interval.
                let mut yield_pending = true;
                futures::future::poll_fn(|cx| {
                    if std::mem::take(&mut yield_pending) {
                        cx.waker().wake_by_ref();
                        std::task::Poll::Pending
                    } else { std::task::Poll::Ready(()) }
                }).await;
                chunks_this_poll = 0;
            }
        }
        decoder.finish();
        match reducer.finish() {
            Ok(output) => Ok(Consumed { head, events, output }),
            Err(error) => Err(ReadFailure { head, events, error: ReadError::Reducer(error) }),
        }
    }
}

/// SSE always uses UTF-8. Only the media-type essence matters, so empty or
/// malformed extension parameters are ignored, as in browser MIME parsing.
/// Controls and comma-joined media types remain ineligible. Commas inside
/// complete quoted parameter values do not make the essence ambiguous.
fn is_sse_media_type(value: &str) -> bool {
    let value = value.trim_matches([' ', '\t']);
    let (base, parameters) = value.split_once(';').unwrap_or((value, ""));
    if !base.trim_matches([' ', '\t']).eq_ignore_ascii_case("text/event-stream")
        || value.bytes().any(|b| (b < b' ' && b != b'\t') || b == 127)
    {
        return false;
    }
    let bytes = parameters.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        // Skip an ignored parameter name or malformed segment. Its comma
        // would still be a header-value separator, not quoted data.
        while index < bytes.len() && !matches!(bytes[index], b';' | b'=') {
            if bytes[index] == b',' { return false; }
            index += 1;
        }
        if index == bytes.len() { break; }
        if bytes[index] == b';' {
            index += 1;
            continue;
        }
        index += 1;
        while bytes.get(index).is_some_and(|b| matches!(*b, b' ' | b'\t')) {
            index += 1;
        }
        if bytes.get(index) == Some(&b'"') {
            index += 1;
            let mut comma = false;
            let mut closed = false;
            while let Some(byte) = bytes.get(index).copied() {
                index += 1;
                if byte == b'"' { closed = true; break; }
                let byte = if byte == b'\\' {
                    let Some(next) = bytes.get(index).copied() else { break };
                    index += 1;
                    next
                } else { byte };
                comma |= byte == b',';
            }
            // An unclosed quote cannot establish that a comma belongs to
            // one parameter instead of a second joined Content-Type value.
            if !closed && comma { return false; }
        }
        while index < bytes.len() && bytes[index] != b';' {
            if bytes[index] == b',' { return false; }
            index += 1;
        }
        if index < bytes.len() { index += 1; }
    }
    true
}
