//! Opt-in downstream ownership bridge. This candidate is not application dispatch.
//!
//! Each body poll forwards the original native item. Cleanup is a shared view
//! of the original owner completion, not a byte-stream or inference result.

use crate::transport::consumer::Consumed;
use crate::transport::http::NativeByteStream;
use crate::transport::http::NativeResponse;
use crate::account::owner::AdmissionError;
use crate::account::owner::Cancellation;
use crate::account::owner::Completion;
use crate::account::owner::Inputs;
use crate::account::owner::RequestOwner;
use crate::account::owner::RequestPool;
use crate::account::owner::ResponseError;
use crate::account::owner::ResponseReceiver;
use crate::transport::response::AdaptedResponse;
use crate::transport::response::Mode;
use crate::transport::response::Rejection;
use crate::transport::terminal::Terminal;
use bytes::Bytes;
use framer_core::port::ProxyResponse;
use futures::future::Shared;
use futures::FutureExt;
use futures::Stream;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use tokio::time::Instant;

/// All clones poll one real Completion and receive its same copied result.
/// Retain a clone before dropping the response future or downstream stream.
pub type Cleanup = Shared<Completion>;

pub type ResponseFuture =
    Pin<Box<dyn Future<Output = Result<BridgedResponse, BridgeError>> + Send + 'static>>;

#[must_use = "consume or drop the response and observe cleanup independently"]
pub struct Pending {
    pub response: ResponseFuture,
    pub cleanup: Cleanup,
    pub cancellation: Cancellation,
}

impl fmt::Debug for Pending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("downstream-owner: pending response and cleanup")
    }
}

/// The existing port cannot represent an absent content type or usage header.
/// Preserve both here. A rejection from an attached terminal-mode owner also
/// retains its original reason while forwarding that rejected raw body.
#[must_use = "consume or drop the body and inspect the independent cleanup result"]
pub struct BridgedResponse {
    pub proxy: ProxyResponse,
    pub usage_header: Option<String>,
    pub original_content_type: Option<String>,
    pub rejection: Option<Rejection>,
}

impl fmt::Debug for BridgedResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgedResponse")
            .field("status", &self.proxy.status)
            .field("has_usage_header", &self.usage_header.is_some())
            .field("has_original_content_type", &self.original_content_type.is_some())
            .field("rejected", &self.rejection.is_some())
            .finish_non_exhaustive()
    }
}

/// These retain the original response error or complete unexpected terminal
/// value. Neither variant converts it into an empty successful raw response.
pub enum BridgeError {
    Response(ResponseError),
    UnexpectedTerminal(Consumed<Terminal>),
}

impl fmt::Debug for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Response(error) => f.debug_tuple("Response").field(error).finish(),
            Self::UnexpectedTerminal(_) => {
                f.write_str("downstream-owner: unexpected terminal response")
            }
        }
    }
}

/// Attachment consumes neither a missing receiver nor any receiver held by
/// another caller. Recover every piece taken here, or drop to cancel the owner.
pub struct AttachError {
    pub owner: RequestOwner,
    pub response: Option<ResponseReceiver>,
    pub completion: Option<Completion>,
}

impl fmt::Debug for AttachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AttachError")
            .field("response_available", &self.response.is_some())
            .field("completion_available", &self.completion.is_some())
            .finish_non_exhaustive()
    }
}

pub enum StartError {
    Admission(AdmissionError),
    Attach(AttachError),
}

impl fmt::Debug for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admission(error) => f.debug_tuple("Admission").field(error).finish(),
            Self::Attach(error) => f.debug_tuple("Attach").field(error).finish(),
        }
    }
}

/// Admit one raw request through the caller's existing shared pool. There is
/// no extra task, admission queue or fresh timeout. The owner's absolute
/// deadline governs send and body work; cleanup still awaits real completion.
///
/// The caller must choose the missing content-type value explicitly. It is
/// used only for None; a present value, including an empty one, is unchanged.
pub fn start(
    pool: &RequestPool,
    inputs: Inputs,
    deadline: Instant,
    missing_content_type: String,
) -> Result<Pending, StartError> {
    let owner = pool.start(inputs, Mode::Raw, deadline).map_err(StartError::Admission)?;
    from_owner(owner, missing_content_type).map_err(StartError::Attach)
}

/// Attach a caller-owned request without changing its mode or deadline.
///
/// Both receivers must still be present. A missing receiver returns the owner
/// and every receiver taken by this call; the caller can observe or cancel
/// without losing a handle. Terminal values stay typed errors, never bytes
/// reconstructed from JSON. Rejected raw responses preserve their reason.
pub fn from_owner(
    mut owner: RequestOwner,
    missing_content_type: String,
) -> Result<Pending, AttachError> {
    let response = owner.take_response();
    let completion = owner.take_completion();
    let (response, completion) = match (response, completion) {
        (Some(response), Some(completion)) => (response, completion),
        (response, completion) => {
            return Err(AttachError { owner, response, completion });
        }
    };
    let cancellation = owner.cancellation();
    let cleanup = completion.shared();
    let response = Box::pin(receive(owner, response, cleanup.clone(), missing_content_type));
    Ok(Pending { response, cleanup, cancellation })
}

async fn receive(
    owner: RequestOwner,
    response: ResponseReceiver,
    cleanup: Cleanup,
    missing_content_type: String,
) -> Result<BridgedResponse, BridgeError> {
    let response = match response.await {
        Ok(response) => response,
        Err(error) => {
            // Even a returned send/response error can precede the real join.
            // Keeping the owner here avoids rewriting its end reason on Drop.
            let _ = cleanup.await;
            drop(owner);
            return Err(BridgeError::Response(error));
        }
    };
    let (response, rejection) = match response {
        AdaptedResponse::Raw(response) => (response, None),
        AdaptedResponse::Rejected { response, reason } => (response, Some(reason)),
        AdaptedResponse::Terminal(terminal) => {
            let _ = cleanup.await;
            drop(owner);
            return Err(BridgeError::UnexpectedTerminal(terminal));
        }
    };
    let NativeResponse { status, usage_header, content_type, body } = response;
    let downstream_content_type = match &content_type {
        Some(value) => value.clone(),
        None => missing_content_type,
    };
    Ok(BridgedResponse {
        proxy: ProxyResponse {
            status,
            content_type: downstream_content_type,
            stream: Box::pin(OwnedBody {
                body: Some(body),
                owner: Some(owner),
                cleanup,
                done: false,
            }),
            // This bridge has no browser-page wait or admission queue.
            queue_wait_ms: 0,
        },
        usage_header,
        original_content_type: content_type,
        rejection,
    })
}

struct OwnedBody {
    body: Option<NativeByteStream>,
    owner: Option<RequestOwner>,
    cleanup: Cleanup,
    done: bool,
}

impl Stream for OwnedBody {
    type Item = Result<Bytes, String>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        if let Some(body) = &mut this.body {
            match Pin::new(body).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(item)) => return Poll::Ready(Some(item)),
                Poll::Ready(None) => drop(this.body.take()),
            }
        }
        // Native EOF alone is not an owner join. Keep the owner until the
        // original acknowledgement and real task completion are both observed.
        // Cleanup failures remain in Cleanup; do not inject synthetic bytes or
        // errors into the otherwise untouched downstream stream.
        match Pin::new(&mut this.cleanup).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(_) => {
                drop(this.owner.take());
                this.done = true;
                Poll::Ready(None)
            }
        }
    }
}

impl Drop for OwnedBody {
    fn drop(&mut self) {
        // If unfinished, owner cancellation first releases the native wire
        // body even if this wrapper or its consumers have stopped polling.
        drop(self.owner.take());
        drop(self.body.take());
    }
}
