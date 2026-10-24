//! Dormant transport for the existing BrowserBridge::ai_post envelope.
//!
//! This does NOT generate Castle tokens. Supply a genuinely fresh token from
//! this account's own browser for EACH request. Native acceptance is unproven.
//! This module does not remove that browser or change billing/access policy.
//! ai_post preserves (status, usage_header, text); ai_post_stream exposes raw
//! response bytes without interpreting provider SSE. HTTP errors stay HTTP
//! responses. Requests are sent once with certificate verification, a truthful
//! User-Agent, no redirects, no cookie jar, and no implicit proxy.

use bytes::Bytes;
use framer_core::ids::{AccountId, FramerUserId};
use framer_core::port::{CastleToken, Session};
use futures::task::AtomicWaker;
use futures::{Future, Stream};
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE, COOKIE, ORIGIN, REFERER};
use serde_json::Value;
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

const CANCELLED: &str = "native-http: cancelled";
const REQUEST_LIMIT: &str = "native-http: request exceeds byte limit";
const RESPONSE_LIMIT: &str = "native-http: response exceeds byte limit";
type WireBody = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

#[path = "closure.rs"]
pub mod closure;
pub use closure::{NativeTrackedRequest, NativeResponseFuture, NativeClosed, NativeCloseError};

#[path = "native_send_evidence.rs"]
pub(crate) mod send_evidence;

#[path = "native_transport_facts.rs"]
mod transport_facts;

/// Headers and an on-demand raw stream. Native HTTP exposes a real usage
/// header even when browser CORS would have hidden it.
pub struct NativeResponse {
    pub status: u16,
    pub usage_header: Option<String>,
    pub content_type: Option<String>,
    pub body: NativeByteStream,
}

/// Per-request control. Cancelling drops a received body immediately, even if
/// the consumer is idle. Before headers, it wakes and cancels the send future.
#[derive(Clone, Default)]
pub struct NativeCancellation {
    inner: Arc<CancellationInner>,
}

#[derive(Default)]
struct CancellationInner {
    cancelled: AtomicBool,
    claimed: AtomicBool,
    waker: AtomicWaker,
    response: Mutex<Option<Weak<Mutex<StreamingState>>>>,
}

impl NativeCancellation {
    pub fn new() -> Self { Self::default() }

    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
        let state = lock(&self.inner.response).as_ref().and_then(Weak::upgrade);
        if let Some(state) = state { closure::finish_and_notify(&state, Some(CANCELLED)); }
        self.inner.waker.wake();
    }

    fn claim(&self) -> Result<(), String> {
        if self.inner.cancelled.load(Ordering::Acquire) { return Err(CANCELLED.into()); }
        self.inner.claimed.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "native-http: cancellation handle already used".to_owned())?;
        Ok(())
    }

    fn cancelled(&self) -> impl Future<Output = ()> + '_ {
        futures::future::poll_fn(|cx| {
            self.inner.waker.register(cx.waker());
            if self.inner.cancelled.load(Ordering::Acquire) { Poll::Ready(()) }
            else { Poll::Pending }
        })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// At most one bounded transport chunk is pending. There is no producer task.
/// Drop closes the unfinished body and releases this account's capacity.
pub struct NativeByteStream {
    state: Arc<Mutex<StreamingState>>,
    cancellation: NativeCancellation,
}

struct StreamingState {
    body: Option<WireBody>,
    pending: Bytes,
    permit: Option<Permit>,
    received: usize,
    limit: usize,
    incoming_limit: usize,
    emitted_limit: usize,
    finished: bool,
    error: Option<&'static str>,
    closure: Option<closure::CompletionSignal>,
}

impl StreamingState {
    fn take_pending(&mut self) -> Bytes {
        let size = self.pending.len().min(self.emitted_limit);
        let chunk = self.pending.split_to(size);
        // Even an empty Bytes slice can retain the complete backing allocation.
        // Release our final reference without requiring another consumer poll.
        if self.pending.is_empty() { self.pending = Bytes::new(); }
        chunk
    }

    fn finish(&mut self, error: Option<&'static str>) {
        if self.finished { return; }
        self.body = None;
        self.pending = Bytes::new();
        self.permit = None;
        self.finished = true;
        self.error = error;
    }
}

impl Drop for NativeByteStream {
    fn drop(&mut self) { closure::finish_and_notify(&self.state, None); }
}

impl Stream for NativeByteStream {
    type Item = Result<Bytes, String>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.cancellation.inner.waker.register(cx.waker());
        let mut state = lock(&self.state);
        let result = (|| {
        if self.cancellation.inner.cancelled.load(Ordering::Acquire) {
            state.finish(Some(CANCELLED));
        }
        if let Some(error) = state.error.take() { return Poll::Ready(Some(Err(error.into()))); }
        if state.finished { return Poll::Ready(None); }
        if !state.pending.is_empty() {
            return Poll::Ready(Some(Ok(state.take_pending())));
        }
        let next = match state.body.as_mut() {
            Some(body) => body.as_mut().poll_next(cx),
            None => return Poll::Ready(None),
        };
        match next {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => { state.finish(None); Poll::Ready(None) }
            Poll::Ready(Some(Err(error))) => {
                let error = transport_error(error);
                state.finish(None);
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(Some(Ok(chunk))) => {
                if chunk.len() > state.incoming_limit {
                    state.finish(None);
                    return Poll::Ready(Some(Err("native-http: transport chunk exceeds byte limit".into())));
                }
                if chunk.len() > state.limit.saturating_sub(state.received) {
                    state.finish(None);
                    return Poll::Ready(Some(Err(RESPONSE_LIMIT.into())));
                }
                state.received += chunk.len();
                // Wire chunks may be tiny views of much larger allocations.
                // Own only the checked visible bytes beyond this poll. Emitted
                // slices can share this bounded allocation, never the wire one.
                state.pending = Bytes::copy_from_slice(&chunk);
                drop(chunk);
                Poll::Ready(Some(Ok(state.take_pending())))
            }
        }
        })();
        let completion = state.take_closed_signal();
        drop(state);
        closure::notify(completion);
        result
    }
}

struct Permit(Arc<AtomicUsize>);

impl Permit {
    fn acquire(count: &Arc<AtomicUsize>, maximum: usize) -> Result<Self, String> {
        count.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            if n < maximum { Some(n + 1) } else { None }
        }).map_err(|_| "native-http: account at capacity".to_owned())?;
        Ok(Self(Arc::clone(count)))
    }
}

impl Drop for Permit {
    fn drop(&mut self) { self.0.fetch_sub(1, Ordering::AcqRel); }
}

fn transport_error(error: reqwest::Error) -> String {
    // Never stringify upstream errors: they may contain a URL or credentials.
    if error.is_timeout() { "native-http: request timed out".into() }
    else { "native-http: transport failed".into() }
}

fn response_header(
    headers: &reqwest::header::HeaderMap,
    name: &'static str,
    limit: usize,
) -> Result<Option<String>, String> {
    let mut out = None::<String>;
    for value in headers.get_all(name).iter() {
        let first = out.is_none();
        let text = out.get_or_insert_with(String::new);
        if !first {
            if limit.saturating_sub(text.len()) < 2 {
                return Err("native-http: response header exceeds byte limit".into());
            }
            text.push_str(", ");
        }
        for byte in value.as_bytes() {
            text.push(char::from(*byte));
            if text.len() > limit { return Err("native-http: response header exceeds byte limit".into()); }
        }
    }
    Ok(out)
}

struct LimitedWriter { bytes: Vec<u8>, limit: usize }

impl LimitedWriter {
    fn new(limit: usize) -> Self { Self { bytes: Vec::new(), limit } }
}

impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "byte limit"));
        }
        let length = self.bytes.len() + bytes.len();
        if length > self.bytes.capacity() {
            let target = length.max(self.bytes.capacity().saturating_mul(2)).min(self.limit);
            self.bytes.try_reserve_exact(target - self.bytes.len())
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "allocation failed"))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

fn envelope(body: &Value, castle: &str, limit: usize) -> Result<Vec<u8>, String> {
    let object = body.as_object().ok_or("native-http: AI envelope must be an object")?;
    let mut out = LimitedWriter::new(limit);
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        out.write_all(b"{")?;
        for (index, (key, value)) in object.iter().enumerate() {
            if index > 0 { out.write_all(b",")?; }
            serde_json::to_writer(&mut out, key)?;
            out.write_all(b":")?;
            if key == "castleToken" { serde_json::to_writer(&mut out, castle)?; }
            else { serde_json::to_writer(&mut out, value)?; }
        }
        if !object.contains_key("castleToken") {
            if !object.is_empty() { out.write_all(b",")?; }
            out.write_all(b"\"castleToken\":")?;
            serde_json::to_writer(&mut out, castle)?;
        }
        out.write_all(b"}")?;
        Ok(())
    })();
    result.map_err(|_| REQUEST_LIMIT.to_owned())?;
    Ok(out.bytes)
}

fn fetch_text(bytes: Vec<u8>, limit: usize) -> Result<String, String> {
    match String::from_utf8(bytes) {
        Ok(mut text) => {
            if text.starts_with('\u{feff}') { text.drain(..3); }
            Ok(text)
        }
        Err(error) => {
            let bytes = error.into_bytes();
            let mut rest = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
            let mut out = LimitedWriter::new(limit);
            while !rest.is_empty() {
                match std::str::from_utf8(rest) {
                    Ok(valid) => {
                        out.write_all(valid.as_bytes()).map_err(|_| RESPONSE_LIMIT.to_owned())?;
                        break;
                    }
                    Err(invalid) => {
                        let valid = invalid.valid_up_to();
                        out.write_all(&rest[..valid]).map_err(|_| RESPONSE_LIMIT.to_owned())?;
                        out.write_all("\u{fffd}".as_bytes()).map_err(|_| RESPONSE_LIMIT.to_owned())?;
                        let skip = invalid.error_len().unwrap_or(rest.len() - valid);
                        rest = &rest[valid + skip..];
                    }
                }
            }
            // Only validated UTF-8 and replacement characters were appended.
            String::from_utf8(out.bytes).map_err(|_| "native-http: text decoding failed".into())
        }
    }
}

/// Finite prototype limits. Over-limit input is rejected, never retried.
#[derive(Clone, Copy, Debug)]
pub struct NativeHttpLimits {
    pub request_bytes: usize,
    pub buffered_response_bytes: usize,
    pub streamed_response_bytes: usize,
    pub incoming_chunk_bytes: usize,
    pub emitted_chunk_bytes: usize,
    pub credential_bytes: usize,
    pub in_flight: usize,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    /// Age since receipt from the browser, not proof of actual token age.
    pub token_max_age: Duration,
}

impl Default for NativeHttpLimits {
    fn default() -> Self {
        Self {
            request_bytes: 4 * 1024 * 1024,
            buffered_response_bytes: 8 * 1024 * 1024,
            streamed_response_bytes: 64 * 1024 * 1024,
            incoming_chunk_bytes: 512 * 1024,
            emitted_chunk_bytes: 16 * 1024,
            credential_bytes: 16 * 1024,
            in_flight: 1,
            connect_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(180),
            token_max_age: Duration::from_secs(10),
        }
    }
}

impl NativeHttpLimits {
    fn validate(self) -> Result<Self, String> {
        if self.request_bytes == 0
            || self.buffered_response_bytes == 0
            || self.streamed_response_bytes < self.buffered_response_bytes
            || self.emitted_chunk_bytes == 0
            || self.incoming_chunk_bytes < self.emitted_chunk_bytes
            || self.credential_bytes == 0
            || self.in_flight == 0
            || self.connect_timeout.is_zero()
            || self.request_timeout < self.connect_timeout
            || self.token_max_age.is_zero()
        {
            return Err("native-http: invalid limits".into());
        }
        Ok(self)
    }
}

/// Bind the existing in-memory session and bearer to their authenticated
/// account. These secrets are borrowed for a call, never client defaults.
pub struct AccountRequestAuth<'a> {
    account: AccountId,
    session: &'a Session,
    bearer: &'a str,
}

impl<'a> AccountRequestAuth<'a> {
    pub fn new(account: AccountId, session: &'a Session, bearer: &'a str) -> Self {
        Self { account, session, bearer }
    }
}

/// Non-Clone, consumed-once browser result. No token pool or replay exists here.
pub struct FreshCastleToken {
    account: AccountId,
    token: CastleToken,
    received_at: Instant,
}

impl FreshCastleToken {
    /// Wrap immediately after a fresh mint in this account's own browser.
    /// The integration must establish provenance; this wrapper cannot prove it.
    pub fn from_browser(account: AccountId, token: CastleToken) -> Self {
        Self { account, token, received_at: Instant::now() }
    }

    /// Preserve the trusted resident owner's timestamp from before acquire.
    /// Context validation does not establish mint provenance or lease cleanup.
    pub(super) fn from_browser_at(account: AccountId, token: CastleToken,
        received_at: Instant) -> Result<Self, String> {
        if received_at > Instant::now() {
            return Err("native-http: invalid browser acquire time".into());
        }
        Ok(Self { account, token, received_at })
    }

    /// Fresh, unchanged output from this account's isolated SDK invocation.
    /// The caller establishes provenance and single use. This constructor
    /// does not establish browser execution or upstream token acceptance.
    pub fn from_sdk_output(account: AccountId, token: CastleToken) -> Self {
        Self { account, token, received_at: Instant::now() }
    }

    /// Preserve the original SDK handoff time across worker cleanup and joins.
    /// Provenance remains the supplier's responsibility; future times fail.
    pub fn from_sdk_output_at(account: AccountId, token: CastleToken,
        received_at: Instant) -> Result<Self, String> {
        if received_at > Instant::now() {
            return Err("native-http: invalid SDK handoff time".into());
        }
        Ok(Self { account, token, received_at })
    }
}

#[cfg(test)]
#[path = "sdk_timestamp_tests.rs"]
mod sdk_timestamp_tests;

/// Each instance belongs permanently to one authenticated account and owns its
/// connection pool. A shared or credential-bearing client cannot be injected.
pub struct NativeHttp {
    account: AccountId,
    active_user: FramerUserId,
    client: reqwest::Client,
    base: String,
    limits: NativeHttpLimits,
    in_flight: Arc<AtomicUsize>,
    explicit_proxy: Option<String>,
    resident_browser_headers: bool,
}

/// Select egress from this account's private configuration. No environment
/// proxy is inherited. Proxy URLs may contain secrets and are never logged.
pub enum NativeEgress<'a> {
    Direct,
    Proxy(&'a str),
}

impl NativeHttp {
    /// Opt in to the serving browser's inference header contract for an
    /// owner-managed comparison. The supplied Session must be its current,
    /// API-selected jar, retained under the same resident-context lease.
    /// This does not acquire that lease or reproduce browser TLS/HTTP behavior.
    pub fn with_resident_browser_headers(mut self) -> Self {
        self.resident_browser_headers = true;
        self
    }

    pub(super) fn matches_egress(&self, egress: &NativeEgress<'_>) -> bool {
        match egress {
            NativeEgress::Direct => self.explicit_proxy.is_none(),
            NativeEgress::Proxy(url) => self.explicit_proxy.as_deref() == Some(*url),
        }
    }

    pub fn new(
        account: AccountId,
        active_user: FramerUserId,
        limits: NativeHttpLimits,
        egress: NativeEgress<'_>,
    ) -> Result<Self, String> {
        Self::build(account, active_user, crate::transport::API.to_owned(), limits, true, egress)
    }

    fn build(
        account: AccountId,
        active_user: FramerUserId,
        base: String,
        limits: NativeHttpLimits,
        https_only: bool,
        egress: NativeEgress<'_>,
    ) -> Result<Self, String> {
        let limits = limits.validate()?;
        let builder = reqwest::Client::builder()
            .user_agent("Framerize/0.1")
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .https_only(https_only)
            // Ordinary HTTP/1 avoids an HTTP/2 prefetch window per stream.
            // It is not a browser TLS profile or impersonation.
            .http1_only()
            .pool_max_idle_per_host(1)
            .pool_idle_timeout(Duration::from_secs(15))
            .connect_timeout(limits.connect_timeout)
            .timeout(limits.request_timeout)
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd();
        let explicit_proxy = match &egress {
            NativeEgress::Direct => None,
            NativeEgress::Proxy(url) => Some((*url).to_owned()),
        };
        let builder = match egress {
            NativeEgress::Direct => builder,
            NativeEgress::Proxy(url) => builder.proxy(reqwest::Proxy::all(url)
                .map_err(|_| "native-http: invalid account proxy".to_owned())?),
        };
        let client = builder.build()
            .map_err(|_| "native-http: client initialization failed".to_owned())?;
        Ok(Self {
            account, active_user, client, base, limits, explicit_proxy,
            in_flight: Arc::new(AtomicUsize::new(0)),
            resident_browser_headers: false,
        })
    }

    /// Test-only loopback origin. Production cannot select HTTP or a new host.
    #[cfg(test)]
    pub(crate) fn for_loopback(
        account: AccountId,
        active_user: FramerUserId,
        address: std::net::SocketAddr,
        limits: NativeHttpLimits,
    ) -> Result<Self, String> {
        if !address.ip().is_loopback() {
            return Err("native-http: fixture address must be loopback".into());
        }
        Self::build(account, active_user, format!("http://{address}"), limits, false, NativeEgress::Direct)
    }

    fn validate_request(
        &self,
        account: &AccountId,
        path: &str,
        auth: &AccountRequestAuth<'_>,
        castle: &FreshCastleToken,
    ) -> Result<(), String> {
        if *account != self.account || auth.account != self.account || castle.account != self.account {
            return Err("native-http: account mismatch".into());
        }
        if auth.session.active_user != self.active_user {
            return Err("native-http: session identity mismatch".into());
        }
        // Only existing inference routes. Utilities, refunds, controls, and
        // arbitrary authenticated URLs are intentionally ineligible.
        if !crate::routes::ALL.iter().any(|route| route.path == path && matches!(route.id,
            "direct-responses" | "direct-anthropic" | "openrouter-responses"
                | "openrouter-messages" | "openrouter-chat")) {
            return Err("native-http: unsupported AI route".into());
        }
        if castle.token.0.is_empty() {
            return Err("castle-v3-unavailable".into());
        }
        if castle.received_at.elapsed() > self.limits.token_max_age {
            return Err("native-http: fresh Castle token required".into());
        }
        if auth.bearer.is_empty() || auth.session.cookie_header.is_empty() {
            return Err("native-http: request credentials missing".into());
        }
        if [auth.bearer.len(), auth.session.cookie_header.len(), castle.token.0.len()]
            .iter().any(|n| *n > self.limits.credential_bytes)
        {
            return Err("native-http: credential exceeds byte limit".into());
        }
        Ok(())
    }

    /// Existing ai_post result, including HTTP error status/body. Decoding
    /// matches Fetch text(): UTF-8 with replacement and one leading BOM removed.
    #[allow(clippy::too_many_arguments)]
    pub async fn ai_post(
        &self,
        account: &AccountId,
        path: &str,
        auth: AccountRequestAuth<'_>,
        body: &Value,
        castle: FreshCastleToken,
        cancellation: NativeCancellation,
    ) -> Result<(u16, Option<String>, String), String> {
        use futures::StreamExt;
        let response = self.send_once(account, path, auth, body, castle, cancellation,
            self.limits.buffered_response_bytes).await?;
        let mut stream = response.body;
        let mut collected = LimitedWriter::new(self.limits.buffered_response_bytes);
        while let Some(chunk) = stream.next().await {
            collected.write_all(&chunk?).map_err(|_| RESPONSE_LIMIT.to_owned())?;
        }
        let text = fetch_text(collected.bytes, self.limits.buffered_response_bytes)?;
        Ok((response.status, response.usage_header, text))
    }

    /// Return headers as soon as they arrive, then raw provider bytes on demand.
    /// No SSE parsing, dialect conversion, or producer task sits behind this.
    #[allow(clippy::too_many_arguments)]
    pub async fn ai_post_stream(
        &self,
        account: &AccountId,
        path: &str,
        auth: AccountRequestAuth<'_>,
        body: &Value,
        castle: FreshCastleToken,
        cancellation: NativeCancellation,
    ) -> Result<NativeResponse, String> {
        self.send_once(account, path, auth, body, castle, cancellation,
            self.limits.streamed_response_bytes).await
    }

    /// Returns response and physical local cleanup separately, without
    /// polling, spawning a task or introducing retries.
    #[allow(clippy::too_many_arguments)]
    pub fn ai_post_stream_tracked<'a>(
        &'a self,
        account: &'a AccountId,
        path: &'a str,
        auth: AccountRequestAuth<'a>,
        body: &'a Value,
        castle: FreshCastleToken,
        cancellation: NativeCancellation,
    ) -> NativeTrackedRequest<'a> {
        closure::track(self.ai_post_stream(
            account, path, auth, body, castle, cancellation,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_once(
        &self,
        account: &AccountId,
        path: &str,
        auth: AccountRequestAuth<'_>,
        body: &Value,
        castle: FreshCastleToken,
        cancellation: NativeCancellation,
        response_limit: usize,
    ) -> Result<NativeResponse, String> {
        self.validate_request(account, path, &auth, &castle)?;
        cancellation.claim()?;
        let permit = Permit::acquire(&self.in_flight, self.limits.in_flight)?;
        let wire = envelope(body, &castle.token.0, self.limits.request_bytes)?;
        let mut bearer = HeaderValue::from_str(&format!("Bearer {}", auth.bearer))
            .map_err(|_| "native-http: invalid authorization header".to_owned())?;
        bearer.set_sensitive(true);
        // Default inference follows the public app's same-origin fetch mode.
        // A resident-browser control explicitly uses its selected current jar.
        let mut cookie = HeaderValue::from_str(&auth.session.cookie_header)
            .map_err(|_| "native-http: invalid cookie header".to_owned())?;
        cookie.set_sensitive(true);
        // Recheck age immediately before dispatch. No mint/retry is attempted.
        self.validate_request(account, path, &auth, &castle)?;
        let content_type = if self.resident_browser_headers {
            "application/json; charset=utf-8"
        } else { "application/json" };
        let request = self.client.post(format!("{}{path}", self.base))
            .header(CONTENT_TYPE, content_type)
            // Inference belongs to the Framer application origin. Keep the
            // truthful native User-Agent and transport; this is not a browser.
            .header(ORIGIN, "https://framer.com")
            // The API is cross-origin. Send only the public application origin,
            // without inventing or disclosing a project page or query string.
            .header(REFERER, "https://framer.com/")
            .header("accept", "*/*")
            .header("accept-encoding", "identity")
            .header(AUTHORIZATION, bearer)
            .body(wire);
        let request = if self.resident_browser_headers {
            request.header(COOKIE, cookie).header("x-requested-by", "Framer")
        } else { request };
        let invocation = send_evidence::invoked()?;
        let send = request.send();
        let send_cancellation = cancellation.clone();
        let cancelled = send_cancellation.cancelled();
        futures::pin_mut!(send, cancelled);
        let response = match futures::future::select(cancelled, send).await {
            futures::future::Either::Left(_) => return Err(CANCELLED.into()),
            futures::future::Either::Right((result, _)) => result.map_err(transport_error)?,
        };
        // Observe the received head even if local encoding/size checks refuse it.
        // These fixed facts grant no acceptance or cleanup authority.
        send_evidence::received(invocation, response.status(), response.version(), response.headers());
        // Do not feed compressed bytes to SSE consumers or allocate an
        // unbounded decompression buffer. The request explicitly asks identity.
        for encoding in response.headers().get_all("content-encoding").iter() {
            let only_identity = encoding.to_str().is_ok_and(|value| {
                value.split(',').all(|coding| coding.trim().eq_ignore_ascii_case("identity"))
            });
            if !only_identity {
                return Err("native-http: unsupported response content encoding".into());
            }
        }
        if response.content_length().is_some_and(|n| n > response_limit as u64) {
            return Err(RESPONSE_LIMIT.into());
        }
        let status = response.status().as_u16();
        let usage_header = response_header(response.headers(), "x-framer-ai-usage",
            self.limits.credential_bytes)?;
        let content_type = response_header(response.headers(), "content-type",
            self.limits.credential_bytes)?;
        let state = Arc::new(Mutex::new(StreamingState {
            body: Some(Box::pin(response.bytes_stream())),
            pending: Bytes::new(),
            permit: Some(permit),
            received: 0,
            limit: response_limit,
            incoming_limit: self.limits.incoming_chunk_bytes,
            emitted_limit: self.limits.emitted_chunk_bytes,
            finished: false,
            error: None,
            closure: None,
        }));
        *lock(&cancellation.inner.response) = Some(Arc::downgrade(&state));
        if cancellation.inner.cancelled.load(Ordering::Acquire) {
            closure::finish_and_notify(&state, Some(CANCELLED));
            return Err(CANCELLED.into());
        }
        Ok(NativeResponse { status, usage_header, content_type,
            body: NativeByteStream { state, cancellation } })
    }
}

#[cfg(test)]
#[path = "fixture_body.rs"]
pub mod fixture_body;

#[cfg(test)]
#[path = "native_http_memory_tests.rs"]
mod memory_tests;

#[cfg(test)]
#[path = "native_send_tests.rs"]
mod send_tests;
