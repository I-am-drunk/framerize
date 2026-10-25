//! Dormant response routing. No dispatch, retries, header synthesis or queue.
use crate::transport::consumer;
use crate::transport::consumer::Consumed;
use crate::transport::consumer::PreparationError;
use crate::transport::consumer::ReadFailure;
use crate::transport::http::NativeClosed;
use crate::transport::http::NativeResponse;
use crate::transport::http::NativeTrackedRequest;
use crate::transport::sse::DecodeError;
use crate::transport::sse::Limits as FramingLimits;
use crate::transport::terminal::Failure;
use crate::transport::terminal::Limits as TerminalLimits;
use crate::transport::terminal::ResponsesReducer;
use crate::transport::terminal::Terminal;
use std::future::Future;
use std::pin::Pin;

pub enum Mode {
    Raw,
    Terminal { framing: FramingLimits, terminal: TerminalLimits },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Rejection {
    NotSuccessfulSse,
    InvalidFraming(DecodeError),
    InvalidTerminalLimits(Failure),
}

pub enum AdaptedResponse {
    /// Original head and body, including non-SSE or non-success responses.
    Raw(NativeResponse),
    /// Original unpolled response. The caller still owns body cleanup.
    Rejected { response: NativeResponse, reason: Rejection },
    /// The disposition inside output must be checked before claiming success.
    Terminal(Consumed<Terminal>),
}

pub enum AdapterError {
    /// The native transport already sanitizes these strings.
    Send(String),
    /// Includes original status, content type and usage header.
    Consume(ReadFailure<Failure>),
}

impl std::fmt::Debug for AdaptedResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Raw(_) => "AdaptedResponse::Raw",
            Self::Rejected { .. } => "AdaptedResponse::Rejected",
            Self::Terminal(_) => "AdaptedResponse::Terminal",
        })
    }
}

impl std::fmt::Debug for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Send(_) => "native-response: send failed",
            Self::Consume(_) => "native-response: consume failed",
        })
    }
}

pub struct AdaptedRequest<'a> {
    pub response: Pin<Box<dyn Future<Output = Result<AdaptedResponse, AdapterError>> + Send + 'a>>,
    /// The original receiver, not a substitute tied to response publication.
    pub closed: NativeClosed,
}

/// Wrap a real tracked request without polling it. The response future owns
/// send/body lifetime; closed continues to acknowledge the original cleanup.
/// Raw/rejected bodies can outlive this future and keep closed pending.
pub fn adapt_request<'a>(request: NativeTrackedRequest<'a>, mode: Mode) -> AdaptedRequest<'a> {
    let NativeTrackedRequest { response, closed } = request;
    let response = Box::pin(async move {
        let response = response.await.map_err(AdapterError::Send)?;
        adapt_response(response, mode).await
    });
    AdaptedRequest { response, closed }
}


/// Adapt an already-owned response independently of request inputs.
/// The caller retains the original NativeClosed and all deadline control.
pub fn adapt_response(response: NativeResponse, mode: Mode)
    -> impl Future<Output = Result<AdaptedResponse, AdapterError>> + Send + 'static
{
    async move {
        let (framing, terminal) = match mode {
            Mode::Raw => return Ok(AdaptedResponse::Raw(response)),
            Mode::Terminal { framing, terminal } => (framing, terminal),
        };
        // Validate the reducer before moving response into PreparedSse,
        // so every preparation rejection can return the unpolled body.
        let reducer = match ResponsesReducer::new(terminal) {
            Ok(reducer) => reducer,
            Err(error) => return Ok(AdaptedResponse::Rejected {
                response, reason: Rejection::InvalidTerminalLimits(error),
            }),
        };
        let prepared = match consumer::prepare_native_sse(response, framing) {
            Ok(prepared) => prepared,
            Err(PreparationError::NotSuccessfulSse(response)) => {
                return Ok(AdaptedResponse::Rejected {
                    response, reason: Rejection::NotSuccessfulSse,
                });
            }
            Err(PreparationError::InvalidLimits { response, error }) => {
                return Ok(AdaptedResponse::Rejected {
                    response, reason: Rejection::InvalidFraming(error),
                });
            }
        };
        prepared.consume(reducer).await
            .map(AdaptedResponse::Terminal).map_err(AdapterError::Consume)
    }
}
