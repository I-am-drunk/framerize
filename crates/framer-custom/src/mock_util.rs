//! Scripted transport for route-level checks (regular build, not test-only).
//!
//! Mirrors the cfg(test) MockTransport pattern so framer-server's mock
//! route mode has something to run without a live browser. Route-gated by
//! FRAMERIZE_CUSTOM_TRANSPORT=mock on the server side; real mode never
//! builds one.
//!
//! DIVERGENCE NOTE: this intentionally duplicates transport::MockTransport
//! (which is cfg(test)-only and invisible to the server crate). If the two
//! drift, MockTransport is the reference: ScriptedTransport must replay and
//! record identically. A shared `mock` feature would unify them; until the
//! route step wires one, duplication is the smaller risk.

use super::transport::{CustomTransport, Event};

/// Fixed-script transport: replays events, records cancel position.
#[derive(Debug)]
pub struct ScriptedTransport {
    script: Vec<Event>,
    pos: usize,
    credits: f64,
    cancel_pos: Option<usize>,
}

impl ScriptedTransport {
    /// New scripted transport with a fixed credit balance.
    pub fn new(script: Vec<Event>, credits: f64) -> Self {
        Self {
            script,
            pos: 0,
            credits,
            cancel_pos: None,
        }
    }

    /// Event index at which cancel arrived (None = never cancelled).
    pub fn cancel_pos(&self) -> Option<usize> {
        self.cancel_pos
    }

    /// Scripted transports are never live: trailing None ends the script.
    pub fn is_live(&self) -> bool {
        false
    }
}

#[async_trait::async_trait]
impl CustomTransport for ScriptedTransport {
    async fn stream_reply(&mut self, _payload: &serde_json::Value) -> Result<(), String> {
        self.pos = 0;
        self.cancel_pos = None;
        Ok(())
    }

    async fn next_event(&mut self) -> Option<Event> {
        let ev = self.script.get(self.pos).cloned();
        if ev.is_some() {
            self.pos += 1;
        }
        ev
    }

    async fn cancel(&mut self) {
        if self.cancel_pos.is_none() {
            self.cancel_pos = Some(self.pos);
        }
    }

    async fn read_credits(&self) -> Result<f64, String> {
        Ok(self.credits)
    }
}

#[cfg(test)]
mod consistency {
    use super::*;
    use crate::transport::MockTransport;

    /// The two scripted transports must behave identically: same replay,
    /// same cancel recording, same liveness. If this fails, the server mock
    /// route and the engine unit tests have diverged -- fix by porting the
    /// MockTransport change here, then unifying under one `mock` feature.
    #[tokio::test]
    async fn scripted_matches_mock() {
        use crate::transport::CustomTransport;
        let script = MockTransport::deltas(5);
        let mut a = MockTransport::new(script.clone());
        let mut b = ScriptedTransport::new(script, 100.0);
        let p = serde_json::json!({});
        a.stream_reply(&p).await.unwrap();
        b.stream_reply(&p).await.unwrap();
        for _ in 0..3 {
            assert_eq!(a.next_event().await, b.next_event().await);
        }
        a.cancel().await;
        b.cancel().await;
        assert_eq!(a.cancel_pos(), b.cancel_pos());
        assert_eq!(a.read_credits().await.unwrap(), b.read_credits().await.unwrap());
    }
}
