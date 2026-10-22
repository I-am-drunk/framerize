use super::*;

#[cfg(test)]
#[path = "contextual_tests.rs"]
mod contextual_tests;

pub(super) struct ScopedBridge {
    pub registry: Arc<Registry>,
    pub state: Arc<state::ScopeState>,
}

#[async_trait::async_trait]
impl BrowserBridge for ScopedBridge {
    async fn prepare_inference_authorization(&self, account: &AccountId,
        request: framer_core::port::BrowserAuthorizationRequest<'_>)
        -> Result<framer_core::port::BrowserAuthorization, String> {
        let value = self.call(account, "prepare_inference_authorization", ||
            json!({"project": request.project.0, "path": request.path,
                "cookie_header": request.cookie_header})).await?;
        take_context_authorization(value, request)
    }

    async fn castle_token(&self, account: &AccountId) -> Result<CastleToken, String> {
        let v = self.call(account, "mint", || json!({})).await?;
        v.as_str().map(|s| CastleToken(s.to_owned()))
            .ok_or_else(|| "sidecar mint: bad result".into())
    }

    async fn auth_begin(&self, account: &AccountId, email: &str) -> Result<(), String> {
        self.call(account, "begin", || json!({"email": email})).await?;
        Ok(())
    }

    async fn auth_complete(&self, account: &AccountId, token: &str)
        -> Result<(String, Value), String> {
        let mut v = self.call(account, "complete", || json!({"token": token})).await?;
        let cookie = match v.get_mut("cookie_header").map(Value::take) {
            Some(Value::String(cookie)) => cookie,
            _ => return Err("sidecar complete: no cookie_header".into()),
        };
        let me = v.get_mut("me").map(Value::take).unwrap_or(Value::Null);
        Ok((cookie, me))
    }

    async fn seed_session(&self, account: &AccountId, cookie: &str) -> Result<(), String> {
        self.check_account(account)?;
        let (name, value) = cookie.split_once('=').ok_or("bad cookie header")?;
        self.call(account, "set_cookie", || json!({"name": name.trim(), "value": value.trim()})).await?;
        Ok(())
    }

    async fn ai_post(&self, account: &AccountId, path: &str, bearer: &str, body: &Value)
        -> Result<(u16, Option<String>, String), String> {
        let v = self.call(account, "ai_post", ||
            json!({"path": path, "bearer": bearer, "body": body})).await?;
        let ai_result::OwnedAiPostResult { status, usage, text, queue_wait_ms } =
            ai_result::take_ai_post_result(v);
        self.state.queue_wait.store(queue_wait_ms, Ordering::Relaxed);
        if status == 0 {
            if text.contains("velocity-paused") { return Ok((0, usage, text)); }
            return Err("castle-v3-unavailable".into());
        }
        if status == -1 { return Err(format!("in-page fetch failed: {text}")); }
        Ok((status as u16, usage, text))
    }

    async fn last_queue_wait_ms(&self, account: &AccountId) -> u64 {
        if self.check_account(account).is_err() { return 0; }
        self.state.queue_wait.load(Ordering::Relaxed)
    }

    async fn set_proxy(&self, account: &AccountId, proxy: Option<String>) -> Result<(), String> {
        self.check_account(account)?;
        let p = self.state.lock_phase();
        if p.sealed { return Err(ObserverError::Sealed.to_string()); }
        if p.started { return Err(ObserverError::UnsupportedMutation.to_string()); }
        *self.state.proxy.lock().unwrap() = proxy;
        Ok(())
    }

    async fn rotate_identity(&self, account: &AccountId) -> Result<(), String> {
        self.check_account(account)?;
        Err(ObserverError::UnsupportedMutation.to_string())
    }
}

struct StopOnDrop { state: Arc<state::ScopeState>, armed: bool }

fn take_context_authorization(mut value: Value,
    request: framer_core::port::BrowserAuthorizationRequest<'_>)
    -> Result<framer_core::port::BrowserAuthorization, String> {
    if value["source"] != "framer_wrapper_direct" || value["path"] != request.path
        || value["project"] != request.project.0 {
        return Err("invalid contextual authorization response".into());
    }
    Ok(framer_core::port::BrowserAuthorization {
        project: request.project.clone(), path: request.path.to_owned(),
        page_url: take_context_string(&mut value, "page_url", 4096)?,
        castle: CastleToken(take_context_string(&mut value, "token", 16 * 1024)?),
        cookie_header: take_context_string(&mut value, "cookie_header", 16 * 1024)?,
    })
}

fn take_context_string(value: &mut Value, key: &str, limit: usize) -> Result<String, String> {
    match value.get_mut(key).map(Value::take) {
        Some(Value::String(text)) if !text.is_empty() && text.len() <= limit => Ok(text),
        _ => Err("invalid contextual authorization response".into()),
    }
}
impl Drop for StopOnDrop {
    fn drop(&mut self) { if self.armed { self.state.stop(); } }
}

impl ScopedBridge {
    fn check_account(&self, account: &AccountId) -> Result<(), String> {
        if account != &self.state.account { return Err(ObserverError::AccountMismatch.to_string()); }
        Ok(())
    }

    async fn call(&self, account: &AccountId, cmd: &'static str,
        fields: impl FnOnce() -> Value + Send) -> Result<Value, String> {
        self.check_account(account)?;
        let guard = self.state.begin(&self.registry).map_err(|e| e.to_string())?;
        let mut fields = fields();
        fields["account"] = json!(account.0.to_string());
        fields["scope"] = json!(self.state.id.to_string());
        fields["application_generation"] = json!(self.state.application.0);
        if let Some(proxy) = self.state.proxy.lock().unwrap().as_ref() {
            fields["proxy"] = json!(proxy);
        }
        let state = self.state.clone();
        let (sent, received) = oneshot::channel();
        // The guard and observer are registered before this worker is spawned.
        tokio::spawn(async move {
            let result = match state.ready().await {
                Ok(c) => match c.call(cmd, fields, Some(&state)).await {
                    Ok(result) => result,
                    Err(e) => Err(e.to_string()),
                },
                Err(e) => Err(e.to_string()),
            };
            let _ = sent.send(result);
            guard.complete();
        });
        let mut cancellation = StopOnDrop { state: self.state.clone(), armed: true };
        match tokio::time::timeout(REQ_TIMEOUT, received).await {
            Ok(Ok(result)) => { cancellation.armed = false; result },
            Ok(Err(_)) => Err(ObserverError::WorkerLost.to_string()),
            Err(_) => Err("sidecar timeout; observed cleanup still owns the account".into()),
        }
    }
}
