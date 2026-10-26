use super::*;
use std::{collections::BTreeSet, sync::Mutex as StdMutex, time::Duration};
use tokio::time::{timeout_at, Instant};

pub(super) const UNCONFIRMED: &str =
    "host-lifecycle-unconfirmed: CDP command completion was not observed";

#[derive(Default)]
pub(super) struct PendingCommands(StdMutex<BTreeSet<u64>>);

impl PendingCommands {
    pub(super) fn record(&self, id: u64) -> Result<(), String> {
        // Record before polling send: a failed flush can still dispatch work.
        self.0.lock().map_err(|_| UNCONFIRMED.to_owned())?.insert(id);
        Ok(())
    }

    fn observe(&self, id: u64) -> Result<(), String> {
        self.0.lock().map_err(|_| UNCONFIRMED.to_owned())?.remove(&id);
        Ok(())
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.lock().is_ok_and(|pending| pending.is_empty())
    }

    pub(super) fn after_exit(&self) -> Result<(), String> {
        // Only a joined owned child/group can substitute for terminal replies.
        self.0.lock().map_err(|_| UNCONFIRMED.to_owned())?.clear();
        Ok(())
    }

    pub(super) fn error(&self, error: String) -> String {
        if self.is_empty() { error } else { UNCONFIRMED.to_owned() }
    }
}

#[cfg(test)]
#[path = "cdp_pending_commands_tests.rs"]
mod tests;

impl CdpInstance {
    pub(super) async fn next_reply(&mut self, deadline: Instant) -> Result<(u64, Value), String> {
        loop {
            let next = timeout_at(deadline, self.ws.next()).await
                .map_err(|_| "cdp eval timed out".to_owned())?
                .ok_or("cdp ws closed")?.map_err(|error| error.to_string())?;
            if let Message::Text(text) = next {
                let value: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
                if let Some(id) = value["id"].as_u64() {
                    if value.get("result").is_none() && value.get("error").is_none() {
                        return Err("cdp reply has no result or error".into());
                    }
                    self.pending.observe(id)?;
                    return Ok((id, value));
                }
            }
        }
    }

    pub(super) async fn observe_pending(&mut self, deadline: Instant) -> Result<(), String> {
        while !self.pending.is_empty() {
            self.next_reply(deadline).await?;
        }
        Ok(())
    }

    pub(super) async fn disconnect_observed(&mut self, wait: Duration) -> Result<(), String> {
        let deadline = Instant::now() + wait;
        self.observe_pending(deadline).await.map_err(|error| self.pending.error(error))?;
        let _ = timeout_at(deadline, self.ws.close(None)).await;
        Ok(())
    }
}
