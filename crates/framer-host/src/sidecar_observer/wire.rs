use super::*;

#[cfg(test)]
#[path = "wire_panic_tests.rs"]
mod panic_tests;

#[derive(Clone, Debug)]
pub(crate) enum RemoteEvent {
    Lost,
    Cleanup { account: AccountId, scope: Uuid, finish: Uuid,
        generation: ApplicationGeneration, clean: bool },
}
impl RemoteEvent {
    pub(crate) fn parse(v: &Value, incarnation: Uuid) -> Option<Self> {
        if v["kind"] != "scope_cleanup" { return None; }
        let parsed = (|| {
            if v["protocol"] != 1 || v["incarnation"].as_str()? != incarnation.to_string()
                || v["extent"] != "account_browser_closed" { return None; }
            let scope = Uuid::parse_str(v["scope"].as_str()?).ok()?;
            let account = AccountId(Uuid::parse_str(v["account"].as_str()?).ok()?);
            let finish = Uuid::parse_str(v["finish"].as_str()?).ok()?;
            let generation = ApplicationGeneration(v["application_generation"].as_u64()?);
            let clean = match v["outcome"].as_str()? {
                "closed" => true, "unconfirmed" => false, _ => return None,
            };
            Some(Self::Cleanup { account, scope, finish, generation, clean })
        })();
        Some(parsed.unwrap_or(Self::Lost))
    }
}

pub(super) struct Connection {
    events: broadcast::Sender<RemoteEvent>,
    pub incarnation: Uuid,
    profile_nonce: Uuid,
    capability: Arc<Mutex<Option<Result<(), ObserverError>>>>,
    limits: ObserverLimits,
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    pending: PendingReplies,
    next_id: Arc<AtomicU64>,
    healthy: Arc<std::sync::atomic::AtomicBool>,
}

pub(super) async fn connect(bridge: &SidecarBridge)
    -> Result<(Arc<Connection>, broadcast::Receiver<RemoteEvent>), ObserverError> {
    let ProcessMode::Observed { profile_nonce, .. } = &bridge.mode else {
        return Err(ObserverError::Unsupported);
    };
    bridge.sidecar().await.map_err(|_| ObserverError::Transport)?;
    let mut guard = bridge.inner.lock().await;
    let s = guard.as_mut().ok_or(ObserverError::GenerationLost)?;
    if !s.healthy.load(Ordering::Acquire) { return Err(ObserverError::GenerationLost); }
    let events = s.events.subscribe();
    let connection = Arc::new(Connection {
        events: s.events.clone(),
        incarnation: s.incarnation, profile_nonce: *profile_nonce,
        capability: s.capability.clone(), stdin: s.stdin.clone(),
        limits: bridge.observed.get().ok_or(ObserverError::Unsupported)?.limits,
        pending: s.pending.clone(),
        next_id: s.next_id.clone(), healthy: s.healthy.clone(),
    });
    Ok((connection, events))
}

impl Connection {
    pub async fn negotiate(&self) -> Result<(), ObserverError> {
        let mut negotiated = self.capability.lock().await;
        if let Some(result) = *negotiated { return result; }
        let fields = json!({"scopes": self.limits.scopes, "commands": self.limits.commands});
        let result = match self.call("observer_hello", fields, None).await {
            Ok(Ok(v)) if v["protocol"] == 1 && v["mode"] == "observed_one_shot_v1"
                && v["incarnation"] == self.incarnation.to_string()
                && v["profile_nonce"] == self.profile_nonce.to_string()
                && v["initially_empty"] == true => Ok(()),
            Ok(_) => Err(ObserverError::Unsupported),
            Err(e) => Err(e),
        };
        *negotiated = Some(result);
        result
    }
    pub fn alive(&self) -> bool { self.healthy.load(Ordering::Acquire) }
    pub fn invalidate(&self) {
        self.healthy.store(false, Ordering::Release);
        fail_pending(&self.pending, "observed generation is unconfirmed");
        let _ = self.events.send(RemoteEvent::Lost);
    }

    pub async fn call(&self, cmd: &str, mut fields: Value,
        stop: Option<&state::ScopeState>)
        -> Result<Result<Value, String>, ObserverError> {
        let cancelled = async {
            if let Some(scope) = stop { scope.stopped().await; }
            else { std::future::pending::<()>().await; }
        };
        let writer = tokio::select! {
            biased;
            _ = cancelled => return Err(ObserverError::Stopped),
            writer = self.stdin.lock() => writer,
        };
        if !self.alive() { return Err(ObserverError::GenerationLost); }
        let id = self.next_id.fetch_update(Ordering::Relaxed, Ordering::Relaxed,
            |n| n.checked_add(1)).map_err(|_| ObserverError::IdentifierExhausted)? + 1;
        fields["id"] = json!(id);
        fields["cmd"] = json!(cmd);
        fields["incarnation"] = json!(self.incarnation.to_string());
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = match self.pending.lock() {
                Ok(pending) => pending,
                Err(poisoned) => {
                    drop(poisoned.into_inner());
                    self.invalidate();
                    return Err(ObserverError::GenerationLost);
                }
            };
            if pending.len() >= 512 { return Err(ObserverError::Capacity); }
            pending.insert(id, tx);
        }
        let _owned = ObservedReply { id, pending: self.pending.clone(),
            healthy: self.healthy.clone(), events: self.events.clone() };
        // This future belongs to an independent scoped worker. It is never
        // cancelled by the response deadline or downstream disconnection.
        let mut frame = FrameGuard { writer, on_lost: || self.invalidate(), complete: false };
        if write_command(&mut *frame.writer, fields).await.is_err() {
            return Err(ObserverError::Transport);
        }
        frame.complete = true;
        drop(frame);
        let result = rx.await.map_err(|_| ObserverError::Transport)?;
        if !self.alive() { return Err(ObserverError::GenerationLost); }
        Ok(result)
    }
}

struct ObservedReply {
    id: u64,
    pending: PendingReplies,
    healthy: Arc<std::sync::atomic::AtomicBool>,
    events: broadcast::Sender<RemoteEvent>,
}
impl Drop for ObservedReply {
    fn drop(&mut self) {
        let poisoned = match self.pending.lock() {
            Ok(mut pending) => { pending.remove(&self.id); false }
            Err(poisoned) => { poisoned.into_inner().remove(&self.id); true }
        };
        if poisoned {
            self.healthy.store(false, Ordering::Release);
            fail_pending(&self.pending, "observed generation is unconfirmed");
            let _ = self.events.send(RemoteEvent::Lost);
        }
    }
}

struct FrameGuard<'a, W, F: Fn()> {
    writer: tokio::sync::MutexGuard<'a, W>,
    on_lost: F,
    complete: bool,
}
impl<W, F: Fn()> Drop for FrameGuard<'_, W, F> {
    fn drop(&mut self) {
        // Close generation admission before releasing a partly written pipe.
        if !self.complete {
            // The real callback closes generation admission before notifying
            // waiters. Destruction must not introduce a second unwind.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.on_lost)()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn interrupted_partial_frame_invalidates_before_unlock() {
        let (writer, mut reader) = tokio::io::duplex(16);
        let pipe = Mutex::new(writer);
        let lost = AtomicBool::new(false);
        let mut task = Box::pin(async {
            let mut frame = FrameGuard { writer: pipe.lock().await, complete: false,
                on_lost: || {
                    assert!(pipe.try_lock().is_err());
                    lost.store(true, Ordering::SeqCst);
                } };
            write_command(&mut *frame.writer, json!({"body": "x".repeat(128)})).await.unwrap();
            frame.complete = true;
        });
        assert!(futures::poll!(task.as_mut()).is_pending());
        drop(task);
        assert!(lost.load(Ordering::SeqCst));
        assert!(pipe.try_lock().is_ok());
        let mut bytes = [0; 16];
        let count = tokio::time::timeout(Duration::from_secs(1), reader.read(&mut bytes))
            .await.unwrap().unwrap();
        assert_eq!(count, 16);
    }

    #[tokio::test]
    async fn complete_frame_does_not_invalidate_the_connection() {
        let (writer, _reader) = tokio::io::duplex(512);
        let pipe = Mutex::new(writer);
        let lost = AtomicBool::new(false);
        let mut frame = FrameGuard { writer: pipe.lock().await, complete: false,
            on_lost: || lost.store(true, Ordering::SeqCst) };
        write_command(&mut *frame.writer, json!({"id": 7})).await.unwrap();
        frame.complete = true;
        drop(frame);
        assert!(!lost.load(Ordering::SeqCst));
    }
}
