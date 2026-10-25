//! Bounded access to the fixed session authorization endpoint.
use crate::transport::Transport;
use framer_core::Session;
use serde_json::Value;

#[derive(Clone, Copy)]
pub enum Egress<'a> { Direct, Proxy(&'a str) }

pub struct Client { transport: Transport }

impl Client {
    pub fn new(egress: Egress<'_>) -> Result<Self, String> {
        Ok(Self { transport: Transport::for_account(egress)? })
    }

    pub async fn ai_token(&self, session: &Session) -> Result<Value, String> {
        self.transport.control_ai_token(session).await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_loopback(address: std::net::SocketAddr, egress: Egress<'_>) -> Result<Self, String> {
        Ok(Self { transport: Transport::control_for_loopback(address, egress)? })
    }
}
