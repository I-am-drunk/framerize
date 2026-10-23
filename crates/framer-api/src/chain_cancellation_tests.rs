use super::{auth::AiToken, freechat, nativechain, transport::Transport};
use async_trait::async_trait;
use framer_core::{AccountId, FramerUserId, ProjectId};
use framer_core::port::{BrowserBridge, CastleToken, ChainConfig, Session};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::Duration;
use tokio::sync::Semaphore;

struct HeldBrowser {
    calls: AtomicUsize, entered: Semaphore, release: Semaphore,
    status: u16, native: bool,
}

#[tokio::test]
async fn responses_cancel_after_current_call_without_retry_or_continuation() {
    check(true).await;
}

#[tokio::test]
async fn messages_cancel_after_current_call_without_retry_or_continuation() {
    check(false).await;
}

async fn check(native: bool) {
    for status in [200, 401, 429, 0, 1] {
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let browser = Arc::new(HeldBrowser { calls: AtomicUsize::new(0),
            entered: Semaphore::new(0), release: Semaphore::new(0), status, native });
        let held = browser.clone();
        let task = tokio::spawn(async move {
            let account = AccountId(uuid::Uuid::from_u128(1));
            let project = ProjectId("fixture-project".into());
            let session = Session { active_user: FramerUserId("fixture-user".into()),
                cookie_header: "fixture-only".into() };
            let cache = Mutex::new(vec![(account, AiToken {
                bearer: "fixture-only".into(), expires_at_ms: i64::MAX })]);
            let transport = Transport::with_base("http://127.0.0.1:9".into());
            let config = ChainConfig { chunk_tokens: 600, max_chunks: 3,
                max_total_tokens: Some(2048), metered_fallback: true,
                cancelled: Some(Arc::new(move || signal.load(Ordering::SeqCst))),
                ..ChainConfig::default() };
            if native {
                nativechain::complete_long(&transport, &cache, &session, held.as_ref(),
                    &account, &project, json!({"model":"gpt-4.1","input":"fixture"}),
                    &config, "fixture-chat", None).await.map(|_| ())
            } else {
                freechat::complete_long(&transport, &cache, &session, held.as_ref(),
                    &account, &project, json!({"model":"claude-sonnet-4",
                        "messages":[{"role":"user","content":"fixture"}]}),
                    &config, "fixture-chat", None).await.map(|_| ())
            }
        });
        tokio::time::timeout(Duration::from_secs(2), browser.entered.acquire())
            .await.unwrap().unwrap().forget();
        cancelled.store(true, Ordering::SeqCst);
        tokio::task::yield_now().await;
        assert!(!task.is_finished(), "cancellation dropped the in-flight call");
        assert_eq!(browser.calls.load(Ordering::SeqCst), 1);
        browser.release.add_permits(1);
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await.expect("cancelled chain started more work").unwrap();
        let error = result.unwrap_err();
        assert_eq!(browser.calls.load(Ordering::SeqCst), 1);
        if status == 200 || status == 401 { assert_eq!(error, "client cancelled"); }
        if status == 1 { assert_eq!(error, "host-lifecycle-unconfirmed: fixture lost"); }
    }
}

#[async_trait]
impl BrowserBridge for HeldBrowser {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        panic!("unexpected standalone mint")
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("unexpected login")
    }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> {
        panic!("unexpected login")
    }
    async fn ai_post(&self, _: &AccountId, _: &str, _: &str, _: &Value)
        -> Result<(u16, Option<String>, String), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.add_permits(1);
        self.release.acquire().await.unwrap().forget();
        if self.status == 0 { return Err("fixture transport failure".into()); }
        if self.status == 1 { return Err("host-lifecycle-unconfirmed: fixture lost".into()); }
        let body = if self.native {
            json!({"id":"fixture-response", "status":"incomplete",
                "incomplete_details":{"reason":"max_output_tokens"},
                "output":[{"type":"message", "id":"fixture-message", "role":"assistant",
                    "content":[{"type":"output_text", "text":"partial"}]}],
                "usage":{"input_tokens":1, "output_tokens":16}})
        } else {
            json!({"id":"fixture-message", "type":"message", "role":"assistant",
                "content":[{"type":"text", "text":"partial"}], "stop_reason":"max_tokens",
                "usage":{"input_tokens":1, "output_tokens":16}})
        };
        Ok((self.status, None, body.to_string()))
    }
}
