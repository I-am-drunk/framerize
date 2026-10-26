use super::*;
use crate::transport::browser::{BrowserRequest, BrowserResponse, BrowserTransport};
use futures::FutureExt;
use std::time::{SystemTime, UNIX_EPOCH};

struct CancelOnDrop(Option<NativeCancellation>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) { if let Some(cancel) = &self.0 { cancel.cancel(); } }
}

impl NativeHttp {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn send_browser(&self, browser: Arc<dyn BrowserTransport>,
        path: &str, auth: AccountRequestAuth<'_>, castle: FreshCastleToken,
        wire: Vec<u8>, cancellation: NativeCancellation, permit: Permit,
        response_limit: usize) -> Result<NativeResponse, String> {
        self.validate_request(&self.account, path, &auth, &castle)?;
        let device = castle.device.as_deref().ok_or("browser transport requires the minted device")?;
        let cookies = crate::authorization::CurrentRequestCookies::from_saved_device(auth.session, device)?
            .into_session(auth.session)?;
        let remaining = self.limits.token_max_age.checked_sub(castle.received_at.elapsed())
            .filter(|v| !v.is_zero()).ok_or("native-http: fresh Castle token required")?;
        let expires = SystemTime::now().checked_add(remaining)
            .ok_or("browser freshness clock overflow")?.duration_since(UNIX_EPOCH)
            .map_err(|_| "browser freshness clock unavailable")?.as_millis();
        let request = BrowserRequest { account: self.account, user: self.active_user.0.clone(),
            cookie_header: cookies.cookie_header, device: device.to_owned(),
            path: path.to_owned(), bearer: auth.bearer.to_owned(), wire,
            proxy: self.explicit_proxy.clone(), expires_unix_ms: expires.try_into()
                .map_err(|_| "browser freshness clock overflow")?,
            timeout_ms: self.limits.request_timeout.as_millis().try_into()
                .map_err(|_| "browser request deadline overflow")?,
            response_limit: response_limit.min(8 * 1024 * 1024) };
        self.run_browser(browser, request, cancellation, permit).await
    }

    async fn run_browser(&self, browser: Arc<dyn BrowserTransport>, request: BrowserRequest,
        cancellation: NativeCancellation, permit: Permit) -> Result<NativeResponse, String> {
        let invocation = send_evidence::invoked()?;
        let mut guard = CancelOnDrop(Some(cancellation.clone()));
        let remote_cancel = cancellation.clone();
        let limit = request.response_limit;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let outcome = browser.send(request, remote_cancel).await;
            if let Ok(response) = &outcome.response {
                send_evidence::browser_received(invocation, response.status);
            }
            let response = if outcome.cleanup_confirmed { outcome.response }
                else { Err("browser cleanup unconfirmed") };
            let _ = sender.send((response, permit));
            outcome.cleanup_confirmed
        });
        *lock(&cancellation.inner.cleanup) = Some(async move {
            match task.await { Ok(true) => Ok(()), _ => Err(NativeCloseError::AcknowledgementLost) }
        }.boxed().shared());
        let (response, permit) = receiver.await.map_err(|_| "browser sender lost")?;
        guard.0 = None;
        self.browser_response(response.map_err(str::to_owned)?, cancellation, permit, limit)
    }

    fn browser_response(&self, response: BrowserResponse, cancellation: NativeCancellation,
        permit: Permit, limit: usize) -> Result<NativeResponse, String> {
        if !(100..=599).contains(&response.status) || response.body.len() > limit
            || [&response.usage, &response.content_type].iter().any(|value|
                value.as_ref().is_some_and(|v| v.len() > self.limits.credential_bytes)) {
            return Err("browser response exceeds contract".into());
        }
        let chunk_limit = self.limits.incoming_chunk_bytes;
        let body = futures::stream::unfold(Bytes::from(response.body), move |mut bytes| async move {
            if bytes.is_empty() { None } else {
                let next = bytes.split_to(bytes.len().min(chunk_limit));
                Some((Ok::<_, reqwest::Error>(next), bytes))
            }
        });
        let state = Arc::new(Mutex::new(StreamingState {
            body: Some(Box::pin(body)), pending: Bytes::new(), permit: Some(permit),
            received: 0, limit, incoming_limit: chunk_limit,
            emitted_limit: self.limits.emitted_chunk_bytes, finished: false,
            error: None, closure: None,
        }));
        *lock(&cancellation.inner.response) = Some(Arc::downgrade(&state));
        if cancellation.is_cancelled() {
            closure::finish_and_notify(&state, Some(CANCELLED));
            return Err(CANCELLED.into());
        }
        Ok(NativeResponse { status: response.status, usage_header: response.usage,
            content_type: response.content_type, body: NativeByteStream { state, cancellation } })
    }
}
