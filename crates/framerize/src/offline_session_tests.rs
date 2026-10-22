use super::*;
use framer_core::{port::{FramerClient, WorkspaceSpec, ProxyResponse, ChainConfig},
    pipeline::{InferenceRequest, InferenceResult},
    convert::{AssembledResponse, AssembledResponses, ChainEvent}, workspace::CreditSnapshot};
use tower::ServiceExt;

pub(super) fn profile() -> serde_json::Value {
    serde_json::json!({"id":"fixture-user",
        "spaces":[{"scope":"organization","ownerId":"sibling-team"}]})
}

struct LoginClient;
#[async_trait::async_trait]
impl FramerClient for LoginClient {
    async fn me(&self, _: &Session) -> Result<serde_json::Value, String> { Ok(profile()) }
    async fn usage(&self, _: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        assert_eq!(team.0, "sibling-team");
        Ok(CreditSnapshot { available: 1000.0, funds: 1, grants: 1, allocated: 1000.0 })
    }
    async fn request_magic_link(&self, _: &str, _: &CastleToken) -> Result<(), String> {
        panic!("unexpected magic link")
    }
    async fn complete_magic_link(&self, _: &str, _: &CastleToken) -> Result<Session, String> {
        panic!("unexpected magic link")
    }
    async fn create_workspace(&self, _: &Session, _: &WorkspaceSpec) -> Result<TeamId, String> {
        panic!("unexpected workspace creation")
    }
    async fn run_inference(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: &InferenceRequest) -> Result<InferenceResult, String> {
        panic!("unexpected inference")
    }
    async fn proxy(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId, _: &ProjectId,
        _: &str, _: serde_json::Value, _: &str) -> Result<ProxyResponse, String> {
        panic!("unexpected proxy")
    }
    async fn freechat_long(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: serde_json::Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>) -> Result<AssembledResponse, String> {
        panic!("unexpected free chat")
    }
    async fn native_long(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: serde_json::Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(ChainEvent) + Send + Sync)>) -> Result<AssembledResponses, String> {
        panic!("unexpected native chat")
    }
}

fn mutation(state: AppState, id: AccountId, login: bool) -> tokio::task::JoinHandle<()> {
    let path = if login { "/api/accounts/complete" } else { "/api/accounts/import" };
    let body = if login {
        serde_json::json!({"account_id":id,"email_token":"fixture","project_id":"sibling-project"})
    } else {
        serde_json::json!({"account_id":id,"session_cookie":"replacement-fixture",
            "project_id":"sibling-project","team_id":"sibling-team"})
    };
    let request = axum::http::Request::builder().method("POST").uri(path)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap())).unwrap();
    tokio::spawn(async move {
        let response = framer_server::router(state, "fixture-no-web-root").oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
    })
}

async fn replacement(login: bool, issuing: bool) {
    for existing in [false, true] {
        let (mut state, ports, spec, sessions) = fixture();
        state.client = Arc::new(LoginClient);
        ports.management.store(true, Ordering::SeqCst);
        let sibling = AccountId::new();
        if existing {
            let mut account = ports.account.lock().unwrap().clone();
            account.id = sibling;
            account.current_team = Some(TeamId("sibling-team".into()));
            account.current_project = Some(ProjectId("sibling-project".into()));
            ports.others.lock().unwrap().insert(sibling, account);
        }
        if issuing { ports.issue.try_acquire().unwrap().forget(); }
        else { ports.read.try_acquire().unwrap().forget(); }
        let run = start(state.clone(), &ports, spec, &sessions);
        wait(&ports.loading).await;
        if issuing { wait(&ports.issuing).await; }
        let mutation = mutation(state.clone(), sibling, login);
        replacement_join(&state, &ports, run, mutation, issuing).await;
        sessions.shutdown(&state).await.unwrap();
    }
}

async fn replacement_join(state: &AppState, ports: &Arc<Ports>, run: Run,
    mutation: tokio::task::JoinHandle<()>, issuing: bool) {
    if issuing {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !ports.cancelled.load(Ordering::SeqCst) { tokio::task::yield_now().await; }
        }).await.unwrap();
        let scope = ports.scope.lock().unwrap().clone().unwrap();
        assert!(scope.closed().now_or_never().is_none());
    } else { wait(&ports.loading).await; }
    assert!(!mutation.is_finished());
    assert_eq!(ports.saves.load(Ordering::SeqCst), 0);
    assert_eq!(ports.stored.lock().unwrap().cookie_header, "session=fixture");
    assert!(run.cleanup.clone().now_or_never().is_none());
    ports.read.add_permits(if issuing { 1 } else { 2 });
    if issuing { ports.issue.add_permits(1); }
    tokio::time::timeout(Duration::from_secs(2), mutation).await.unwrap().unwrap();
    let outcome = finish(run).await;
    assert!(outcome.cleanup_confirmed && outcome.response.is_none() && outcome.failure.is_some());
    assert_eq!(ports.builds.load(Ordering::SeqCst), usize::from(issuing));
    assert_eq!(ports.saves.load(Ordering::SeqCst), 1);
    assert_eq!(ports.stored.lock().unwrap().cookie_header, "session=replacement-fixture");
    assert_eq!(state.state.list().await.unwrap().len(), 2);
}

#[tokio::test]
async fn same_user_import_during_load_rejects_old_generation() { replacement(false, false).await; }

#[tokio::test]
async fn same_user_login_during_load_rejects_old_generation() { replacement(true, false).await; }

#[tokio::test]
async fn same_user_import_during_issuance_waits_for_cleanup() { replacement(false, true).await; }

#[tokio::test]
async fn same_user_login_during_issuance_waits_for_cleanup() { replacement(true, true).await; }

#[tokio::test]
async fn different_store_cannot_bypass_the_shared_session_owner() {
    let (mut state, ports, spec, sessions) = fixture();
    state.sessions = ports.clone();
    let result = launch_with_source(state, spec, Arc::new(Builder(ports.clone())), sessions);
    assert_eq!(result.err().unwrap().kind, FailureKind::InvalidScope);
    assert!(ports.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn account_completion_keeps_shared_store_available_for_next_request() {
    let (state, ports, spec, sessions) = fixture();
    let next = Specification { identity: spec.identity.clone(), route: spec.route,
        payload: spec.payload.clone(), context: AttemptContext {
            feature_category: spec.context.feature_category.clone(),
            feature_name: spec.context.feature_name.clone(), step: spec.context.step.clone(),
            turn_id: spec.context.turn_id.clone(), user_prompt: spec.context.user_prompt.clone(),
        },
        deadline: spec.deadline, token_limit: spec.token_limit };
    let first = finish(start(state.clone(), &ports, spec, &sessions)).await;
    assert!(first.cleanup_confirmed);
    ports.read.add_permits(1);
    ports.issue.add_permits(1);
    let second = finish(start(state.clone(), &ports, next, &sessions)).await;
    assert!(second.cleanup_confirmed);
    assert_eq!(ports.builds.load(Ordering::SeqCst), 2);
    sessions.shutdown(&state).await.unwrap();
}
