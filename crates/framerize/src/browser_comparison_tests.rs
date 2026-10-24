use super::*;
use super::super::comparison as pair;
use framer_host::sidecar::{SidecarBridge, observer::{ObserverFactory, ObserverLimits}};

struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) { std::fs::remove_dir_all(&self.0).unwrap(); }
}
fn observer() -> (Temp, ObserverFactory) {
    let root = std::env::temp_dir().join(format!("comparison-unused-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("observed_sidecar.py");
    std::fs::write(&script, "raise AssertionError('must not start')\n").unwrap();
    let bridge = Arc::new(SidecarBridge::new_observed_one_shot(&root, &script).unwrap());
    let factory = bridge.observer_factory(ObserverLimits { scopes: 1, commands: 2,
        cleanup_timeout: std::time::Duration::from_secs(1) }).unwrap();
    (Temp(root), factory)
}
struct Refused(AtomicUsize);
impl pair::ComparisonWindow for Refused {
    fn admit(&self) -> FutureResult<'_, Result<Selection, Failure>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(Failure::Cooling) })
    }
    fn record(&self, _: &pair::Report) -> FutureResult<'_, Result<(), Failure>> {
        panic!("no publication without admitted exclusion")
    }
}

#[tokio::test]
async fn reused_turn_is_rejected_before_comparison_admission() {
    let (_root, observer) = observer();
    let id = identity();
    let (store, _, _) = fixture(&id, None);
    let window = Arc::new(Refused(AtomicUsize::new(0)));
    let result = pair::launch(spec(id), context().turn_id, pair::Dependencies {
        authority: window.clone(), store: store.clone(), observer,
        permit: Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap(),
    });
    assert!(matches!(result, Err(Failure::Context)));
    assert_eq!(window.0.load(Ordering::SeqCst), 0);
    assert_eq!(store.loads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn comparison_cooldown_refusal_precedes_private_load_and_browser_work() {
    let (_root, observer) = observer();
    let id = identity();
    let (store, _, _) = fixture(&id, None);
    let window = Arc::new(Refused(AtomicUsize::new(0)));
    let waiter = pair::launch(spec(id), "baseline-turn".into(), pair::Dependencies {
        authority: window.clone(), store: store.clone(), observer,
        permit: Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap(),
    }).unwrap();
    let report = tokio::time::timeout(std::time::Duration::from_secs(2), waiter).await.unwrap();
    assert_eq!(report.failure, Some(Failure::Cooling));
    assert!(report.baseline.is_none() && report.native.is_none());
    assert_eq!(window.0.load(Ordering::SeqCst), 1);
    assert_eq!(store.loads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn comparison_envelope_uses_native_mapping_and_exact_turn_metadata() {
    let id = identity();
    let client = NativeAccountClient::new().unwrap();
    let scope = client.open_account(id.account, Session { active_user: id.user.clone(),
        cookie_header: "session=fixture-only".into() }, NativeHttpLimits::default(),
        NativeEgress::Direct).unwrap();
    let mut request = spec(id.clone());
    request.payload["session_id"] = serde_json::json!("must-be-replaced");
    let plan = Plan::new(scope.clone(), request).unwrap();
    let body = plan.comparison_body().unwrap();
    assert_eq!(body["projectId"], id.project.0);
    assert_eq!(body["sessionId"], id.chat_session);
    assert_eq!(body["request"]["session_id"], body["sessionId"]);
    assert_eq!(body["turnId"], context().turn_id);
    assert_eq!(body["request"]["stream"], false);
    assert_eq!(body["request"]["max_output_tokens"], 32);
    assert!(body.get("castleToken").is_none());
    client.close_account(&scope).unwrap().await.unwrap();
    client.shutdown_idle().await.unwrap();
}
