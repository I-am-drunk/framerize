use super::{auth::AiToken, chain::{messages, responses}, transport::Transport};
use async_trait::async_trait;
use framer_core::{AccountId, FramerUserId, ProjectId};
use framer_core::port::{BrowserBridge, CastleToken, ChainConfig, Session};
use serde_json::{json, Value};
use std::sync::{atomic::{AtomicUsize, Ordering}, Mutex};

struct CoolingBrowser {
    calls: AtomicUsize,
    first_transport_error: bool,
}

fn check_chain(native: bool) {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        for first_transport_error in [false, true] {
            let account = AccountId(uuid::Uuid::from_u128(1));
            let project = ProjectId("fixture-project".into());
            let session = Session { active_user: FramerUserId("fixture-user".into()), cookie_header: "fixture-only".into() };
            let cache = Mutex::new(vec![(account, AiToken { bearer: "fixture-only".into(), expires_at_ms: i64::MAX })]);
            let browser = CoolingBrowser { calls: AtomicUsize::new(0), first_transport_error };
            let transport = Transport::with_base("http://127.0.0.1:9".into());
            let config = ChainConfig { chunk_tokens: 1500, metered_fallback: true, ..ChainConfig::default() };
            let result = if native {
                responses::complete_long(&transport, &cache, &session, &browser, &account, &project,
                    json!({"model":"gpt-6-astra","input":"fixture"}), &config, "fixture-chat", None).await.map(|_| ())
            } else {
                messages::complete_long(&transport, &cache, &session, &browser, &account, &project,
                    json!({"model":"claude-sonnet-4","messages":[{"role":"user","content":"fixture"}]}),
                    &config, "fixture-chat", None).await.map(|_| ())
            };
            assert_eq!(result.unwrap_err(), "account-cooling:584");
            assert_eq!(browser.calls.load(Ordering::SeqCst), if first_transport_error { 2 } else { 1 });
        }
    });
}

#[test]
fn native_cooldown_never_retries_capacity_or_falls_back_to_metered() { check_chain(true); }

#[test]
fn anthropic_cooldown_never_retries_capacity_or_falls_back_to_metered() { check_chain(false); }

#[async_trait]
impl BrowserBridge for CoolingBrowser {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> { panic!("unexpected mint") }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> { panic!("unexpected login") }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> { panic!("unexpected login") }
    async fn ai_post(&self, _: &AccountId, _: &str, _: &str, _: &Value) -> Result<(u16, Option<String>, String), String> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.first_transport_error && call == 0 {
            return Err("fixture transport error".into());
        }
        Ok((0, None, "velocity-paused: every device is cooling; retry in 583s".into()))
    }
}
