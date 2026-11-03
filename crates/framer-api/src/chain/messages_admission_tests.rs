use super::*;
use std::sync::Arc;
use std::time::Duration;

#[derive(Default)]
struct Admission {
    refusals: AtomicUsize,
    resume_at: Mutex<Option<tokio::time::Instant>>,
}

#[async_trait]
impl framer_core::port::ChainAdmission for Admission {
    async fn before_attempt(&self) -> Result<(), String> {
        if let Some(deadline) = *self.resume_at.lock().unwrap() {
            assert!(tokio::time::Instant::now() >= deadline, "retry before cooldown");
        }
        Ok(())
    }
    async fn refused(&self, cause: &str) -> Result<(), String> {
        assert_eq!(cause, "risk-refused");
        self.refusals.fetch_add(1, Ordering::SeqCst);
        *self.resume_at.lock().unwrap() = Some(tokio::time::Instant::now() + Duration::from_secs(2));
        Err("account-cooling:2; fixture security pause".into())
    }
}

fn refusal() -> Reply {
    Reply { streaming: false, status: 403, result: Ok("Forbidden".into()) }
}

fn configured(replies: Vec<Reply>, guard: Arc<Admission>) -> Observed {
    run_with_config(replies, ChainConfig { max_total_tokens: Some(4096),
        admission: Some(guard), ..Default::default() })
}

#[test]
fn temporary_refusal_waits_then_retries_the_same_unpublished_attempt() {
    let guard = Arc::new(Admission::default());
    let text = json!({"type":"text","text":"complete"});
    let observed = configured(vec![refusal(), refusal(), buffered(text.clone(), "end_turn")], guard.clone());
    assert!(observed.result.is_ok());
    assert_eq!(guard.refusals.load(Ordering::SeqCst), 2);
    assert_eq!(observed.emitted, vec![text]);
    assert_eq!(observed.requests.len(), 3);
    for request in &observed.requests[1..] {
        assert_eq!(request["request"], observed.requests[0]["request"]);
        assert_eq!(request["projectId"], observed.requests[0]["projectId"]);
    }
    assert_eq!(observed.refunds, 0);
}

#[test]
fn refusal_after_a_checkpoint_never_replays_the_published_prefix() {
    let guard = Arc::new(Admission::default());
    let text = json!({"type":"text","text":"prefix"});
    let final_text = json!({"type":"text","text":"suffix"});
    let observed = configured(vec![buffered(text.clone(), "max_tokens"), refusal(),
        buffered(final_text.clone(), "end_turn")], guard.clone());
    assert_eq!(observed.result.unwrap().content, vec![json!({"type":"text","text":"prefixsuffix"})]);
    assert_eq!(observed.requests.len(), 3);
    assert_eq!(observed.requests[1]["request"], observed.requests[2]["request"]);
    assert_eq!(observed.emitted, vec![text, final_text]);
    assert_eq!(guard.refusals.load(Ordering::SeqCst), 1);
}

#[test]
fn admission_guard_does_not_replay_unknown_or_permanent_failures() {
    for (status, body) in [(400, "invalid field"), (403, "provider permission denied"),
        (403, "This model is forbidden"), (403, "Forbidden in your region"),
        (403, r#"{"error":{"type":"permission_error","message":"Forbidden"}}"#),
        (504, "upstream completion unknown")] {
        let guard = Arc::new(Admission::default());
        let observed = configured(vec![Reply { streaming:false, status,
            result:Ok(body.into()) }], guard.clone());
        assert!(observed.result.is_err());
        assert_eq!(observed.requests.len(), 1);
        assert_eq!(guard.refusals.load(Ordering::SeqCst), 0);
        assert!(observed.emitted.is_empty());
    }
}
