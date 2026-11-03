use super::{auth::AiToken, chain::{messages, responses}, transport::Transport};
use async_trait::async_trait;
use framer_core::{AccountId, FramerUserId, ProjectId};
use framer_core::port::{BrowserBridge, CastleToken, ChainConfig, Session};
use serde_json::{json, Value};
use std::sync::{atomic::{AtomicUsize, Ordering}, Mutex};

struct CoolingBrowser {
    calls: AtomicUsize,
    first_transport_error: bool,
    scenario: Option<Scenario>,
}

struct Scenario {
    responses: bool,
    transport_retry: bool,
    terminal_risk: bool,
    requests: Mutex<Vec<(tokio::time::Instant, Value)>>,
}

fn check_chain(native: bool) {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        for first_transport_error in [false, true] {
            let account = AccountId(uuid::Uuid::from_u128(1));
            let project = ProjectId("fixture-project".into());
            let session = Session { active_user: FramerUserId("fixture-user".into()), cookie_header: "fixture-only".into() };
            let cache = Mutex::new(vec![(account, AiToken::for_session(
                &session, "fixture-only".into(), i64::MAX))]);
            let browser = CoolingBrowser { calls: AtomicUsize::new(0), first_transport_error, scenario:None };
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
            if !native && first_transport_error {
                assert!(result.unwrap_err().starts_with(framer_core::error::ADAPTIVE_INCOMPLETE_PREFIX));
                assert_eq!(browser.calls.load(Ordering::SeqCst), 1);
                continue;
            }
            assert_eq!(result.unwrap_err(), "account-cooling:584; every device is cooling");
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
    async fn ai_post(&self, account: &AccountId, _: &str, _: &str, body: &Value) -> Result<(u16, Option<String>, String), String> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(scenario) = &self.scenario {
            assert_eq!(*account, AccountId(uuid::Uuid::from_u128(1)));
            return scenario.forward(call, body);
        }
        if self.first_transport_error && call == 0 {
            return Err("fixture transport error".into());
        }
        Ok((0, None, "velocity-paused: every device is cooling; retry in 583s".into()))
    }
}

impl Scenario {
    fn forward(&self, call: usize, body: &Value) -> Result<(u16, Option<String>, String), String> {
        self.requests.lock().unwrap().push((tokio::time::Instant::now(), body.clone()));
        let refused = 1 + usize::from(self.transport_retry);
        if self.transport_retry && call == 1 { return Err("fixture transport error".into()); }
        if call == refused {
            return Ok((0, None, "velocity-paused: every device is cooling; retry in 599s".into()));
        }
        if self.terminal_risk && call == refused + 1 { return Ok((403, None, "Forbidden".into())); }
        let chunk = if call == 0 { 0 } else { call - refused };
        let text = ["first", "second", "third"][chunk];
        let last = chunk == 2;
        let mut reply = json!({"id":"fixture-reply", "created_at":1,
            "usage":{"input_tokens":100, "output_tokens":100}});
        if self.responses {
            reply["output"] = json!([{"type":"message", "role":"assistant",
                "content":[{"type":"output_text", "text":text}]}]);
            reply["status"] = json!(if last { "completed" } else { "incomplete" });
            if !last { reply["incomplete_details"] = json!({"reason":"max_output_tokens"}); }
        } else {
            reply["content"] = json!([{"type":"text", "text":text}]);
            reply["stop_reason"] = json!(if last { "end_turn" } else { "max_tokens" });
        }
        Ok((200, None, reply.to_string()))
    }
}

#[tokio::test(start_paused = true)]
async fn midstream_cooldown_preserves_both_dialects_and_failure_classification() {
    for responses in [false, true] {
        for transport_retry in [false, true] {
            for terminal_risk in [false, true] {
                let account = AccountId(uuid::Uuid::from_u128(1));
                let project = ProjectId("fixture-project".into());
                let session = Session { active_user:FramerUserId("fixture-user".into()),
                    cookie_header:"fixture-only".into() };
                let cache = Mutex::new(vec![(account, AiToken::for_session(
                    &session, "fixture-only".into(), i64::MAX))]);
                let browser = CoolingBrowser { calls:AtomicUsize::new(0), first_transport_error:false,
                    scenario:Some(Scenario { responses, transport_retry, terminal_risk,
                        requests:Mutex::new(Vec::new()) }) };
                // Use the deployed default; optional paid fallback has separate behavior.
                let config = ChainConfig { max_chunks:8, metered_fallback:false, ..ChainConfig::default() };
                let events = Mutex::new(String::new());
                let callback = |event| {
                    use framer_core::convert::ChainEvent;
                    let texts: Vec<String> = match event {
                        ChainEvent::Chunk(blocks) => blocks.iter()
                            .map(|b| b["text"].as_str().unwrap().to_owned()).collect(),
                        ChainEvent::ResponsesItems { items, .. } => items.iter()
                            .map(|b| b["content"][0]["text"].as_str().unwrap().to_owned()).collect(),
                        _ => Vec::new(),
                    };
                    events.lock().unwrap().push_str(&texts.concat());
                };
                let transport = Transport::with_base("http://127.0.0.1:9".into());
                let result = if responses {
                    responses::complete_long(&transport, &cache, &session, &browser, &account, &project,
                        json!({"model":"gpt-6-astra","input":"fixture"}), &config,
                        "fixture-chat", Some(&callback)).await.map(|r| (r.chunks, r.output_tokens))
                } else {
                    messages::complete_long(&transport, &cache, &session, &browser, &account, &project,
                        json!({"model":"claude-sonnet-4","messages":[{"role":"user","content":"fixture"}]}),
                        &config, "fixture-chat", Some(&callback)).await.map(|r| (r.chunks, r.output_tokens))
                };
                if !responses && transport_retry {
                    assert!(result.unwrap_err().starts_with(framer_core::error::ADAPTIVE_INCOMPLETE_PREFIX));
                    assert_eq!(*events.lock().unwrap(), "first");
                    assert_eq!(browser.calls.load(Ordering::SeqCst), 2);
                    continue;
                }
                if terminal_risk { assert!(result.unwrap_err().contains("risk-refused")); }
                else { assert_eq!(result.unwrap(), (3, 300)); }
                assert_eq!(*events.lock().unwrap(), if terminal_risk { "first" } else { "firstsecondthird" });
                let requests = browser.scenario.as_ref().unwrap().requests.lock().unwrap();
                let refused = 1 + usize::from(transport_retry);
                assert_eq!(requests.len(), refused + if terminal_risk { 2 } else { 3 });
                assert_eq!(requests[refused].1["request"], requests[refused + 1].1["request"]);
                assert_eq!(requests[refused].1["stepIndex"], requests[refused + 1].1["stepIndex"]);
                assert_eq!(requests[refused].1["sessionId"], requests[refused + 1].1["sessionId"]);
                assert_eq!(requests[refused + 1].0 - requests[refused].0, std::time::Duration::from_secs(600));
            }
        }
    }
}
