//! Handler admission fixtures. These ports never perform external I/O.
#[path = "explicit_account_admission_tests.rs"]
mod explicit_account_admission;
#[path = "api_run_ownership_tests.rs"]
mod api_run_ownership;
#[path = "api_run_capacity_tests.rs"]
mod api_run_capacity;
#[path = "local_request_completion_tests.rs"]
mod local_request_completion;
#[path = "account_import_identity_tests.rs"]
mod import_identity;
#[path = "account_dedupe_retention_tests.rs"]
mod dedupe_retention;
#[path = "account_dedupe_preview_tests.rs"]
mod dedupe_preview;
#[path = "account_provisioning_preview_tests.rs"]
mod provisioning_preview;
#[path = "account_admission_project_scope_tests.rs"]
mod admission_project_scope;
#[path = "account_secondary_workspace_claims_tests.rs"]
mod secondary_workspace_claims;
#[path = "account_registration_project_scope_tests.rs"]
mod registration_project_scope;
#[path = "account_admission_failure_state_tests.rs"]
mod admission_failure_state;
#[path = "account_workspace_scan_tests.rs"]
mod workspace_scan;
#[path = "account_primary_workspace_claim_tests.rs"]
mod primary_workspace_claim;
#[path = "account_secondary_workspace_claim_tests.rs"]
mod secondary_workspace_claim;
#[path = "account_admission_refusal_publication_tests.rs"]
mod admission_refusal_publication;
#[path = "account_registration_identity_scope_tests.rs"]
mod registration_identity_scope;
#[path = "account_registration_missing_team_tests.rs"]
mod registration_missing_team;


use super::{account, policy};
use crate::{ApiError, AppState};
use axum::{extract::State, http::HeaderMap, Json};
use framer_core::account::ManagedAccount;
use framer_core::convert::{AssembledResponse, AssembledResponses, ChainEvent, ClientDialect};
use framer_core::ids::{AccountId, FramerUserId, ProjectId, TeamId};
use framer_core::pipeline::{InferenceRequest, InferenceResult};
use framer_core::port::{BrowserBridge, CastleToken, ChainConfig, FramerClient, ProxyResponse, Session, SessionStore, StateStore, WorkspaceSpec};
use framer_core::workspace::CreditSnapshot;
use futures::future::BoxFuture;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct FixturePorts {
    import: bool,
    workspaces: Mutex<Vec<(&'static str, f64)>>,
    records: Mutex<Vec<ManagedAccount>>,
    saved: Mutex<Vec<Session>>,
    registered_proxies: Mutex<Vec<(AccountId, Option<String>)>>,
    browser_calls: AtomicUsize,
    client_calls: AtomicUsize,
    session_calls: AtomicUsize,
    state_reads: AtomicUsize,
    state_writes: AtomicUsize,
    native_session: Mutex<Option<Option<Session>>>,
    run_reply: Mutex<Option<InferenceResult>>,
    run_error: Mutex<Option<String>>,
    run_calls: Mutex<Vec<(FramerUserId, AccountId, ProjectId)>>,
}

impl FixturePorts {
    fn unexpected(&self, port: &str) -> ! {
        match port {
            "browser" => &self.browser_calls,
            "client" => &self.client_calls,
            "session" => &self.session_calls,
            _ => &self.state_writes,
        }.fetch_add(1, Ordering::SeqCst);
        panic!("handler touched forbidden {port} I/O");
    }

    fn assert_rejected_before_io(&self, reads: usize) {
        assert_eq!(self.browser_calls.load(Ordering::SeqCst), 0);
        assert_eq!(self.client_calls.load(Ordering::SeqCst), 0);
        assert_eq!(self.session_calls.load(Ordering::SeqCst), 0);
        assert_eq!(self.state_writes.load(Ordering::SeqCst), 0);
        assert_eq!(self.state_reads.load(Ordering::SeqCst), reads);
    }
}

// Explicit boxed-future signatures match the existing async-trait ports.
// Keeping the adapter here avoids adding a dependency solely for test mocks.
macro_rules! denied {
    ($port:literal, $name:ident <$($lt:lifetime),*> ($($arg:ident: $ty:ty),*) -> $output:ty) => {
        fn $name<'s, $($lt,)* 'a>(&'s self, $($arg: $ty),*) -> BoxFuture<'a, $output>
        where 's: 'a, $($lt: 'a,)* Self: 'a {
            Box::pin(async move { self.unexpected($port) })
        }
    };
}

impl BrowserBridge for FixturePorts {
    fn register_account_proxy<'s, 'b, 'a>(&'s self, account: &'b AccountId, proxy: Option<String>)
        -> BoxFuture<'a, Result<(), String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            if !self.import { self.unexpected("browser"); }
            self.registered_proxies.lock().unwrap().push((*account, proxy));
            Ok(())
        })
    }
    denied!("browser", castle_token<'b>(_account: &'b AccountId) -> Result<CastleToken, String>);
    denied!("browser", auth_begin<'b, 'c>(_account: &'b AccountId, _email: &'c str) -> Result<(), String>);
    denied!("browser", auth_complete<'b, 'c>(_account: &'b AccountId, _token: &'c str) -> Result<(String, Value), String>);
    denied!("browser", set_proxy<'b>(_account: &'b AccountId, _proxy: Option<String>) -> Result<(), String>);
    denied!("browser", seed_session<'b, 'c>(_account: &'b AccountId, _cookie: &'c str) -> Result<(), String>);
    denied!("browser", ai_post<'b, 'c, 'd, 'e>(_account: &'b AccountId, _path: &'c str, _bearer: &'d str, _body: &'e Value) -> Result<(u16, Option<String>, String), String>);
    denied!("browser", last_queue_wait_ms<'b>(_account: &'b AccountId) -> u64);
    denied!("browser", rotate_identity<'b>(_account: &'b AccountId) -> Result<(), String>);
}

impl FramerClient for FixturePorts {
    fn me<'s, 'b, 'a>(&'s self, session: &'b Session) -> BoxFuture<'a, Result<Value, String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            if !self.import { self.unexpected("client"); }
            self.client_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(session.cookie_header, "session=fixture-only");
            let spaces: Vec<_> = self.workspaces.lock().unwrap().iter()
                .map(|(team, _)| json!({"scope": "organization", "ownerId": team}))
                .collect();
            Ok(json!({"id": "user-2", "spaces": spaces}))
        })
    }

    fn usage<'s, 'b, 'c, 'a>(&'s self, _session: &'b Session, team: &'c TeamId) -> BoxFuture<'a, Result<CreditSnapshot, String>>
    where 's: 'a, 'b: 'a, 'c: 'a, Self: 'a {
        Box::pin(async move {
            if !self.import { self.unexpected("client"); }
            self.client_calls.fetch_add(1, Ordering::SeqCst);
            let available = self.workspaces.lock().unwrap().iter()
                .find(|(id, _)| *id == team.0.as_str())
                .map(|(_, available)| *available)
                .expect("unexpected fixture workspace");
            Ok(CreditSnapshot { available, funds: 1, grants: 1, allocated: available })
        })
    }

    denied!("client", request_magic_link<'b, 'c>(_email: &'b str, _castle: &'c CastleToken) -> Result<(), String>);
    denied!("client", complete_magic_link<'b, 'c>(_token: &'b str, _castle: &'c CastleToken) -> Result<Session, String>);
    fn run_inference<'s, 'b, 'c, 'd, 'e, 'f, 'a>(&'s self,
        session: &'b Session, _browser: &'c dyn BrowserBridge, account: &'d AccountId,
        project: &'e ProjectId, _req: &'f InferenceRequest)
        -> BoxFuture<'a, Result<InferenceResult, String>>
    where 's: 'a, 'b: 'a, 'c: 'a, 'd: 'a, 'e: 'a, 'f: 'a, Self: 'a {
        Box::pin(async move {
            if let Some(error) = self.run_error.lock().unwrap().take() {
                self.client_calls.fetch_add(1, Ordering::SeqCst);
                self.run_calls.lock().unwrap().push((
                    session.active_user.clone(), *account, project.clone(),
                ));
                return Err(error);
            }
            let reply = self.run_reply.lock().unwrap().take()
                .unwrap_or_else(|| self.unexpected("client"));
            self.client_calls.fetch_add(1, Ordering::SeqCst);
            self.run_calls.lock().unwrap().push((
                session.active_user.clone(), *account, project.clone(),
            ));
            Ok(reply)
        })
    }
    denied!("client", proxy<'b, 'c, 'd, 'e, 'f, 'g>(_session: &'b Session, _browser: &'c dyn BrowserBridge, _account: &'d AccountId, _project: &'e ProjectId, _dialect: &'f str, _payload: Value, _chat: &'g str) -> Result<ProxyResponse, String>);
    denied!("client", freechat_long<'b, 'c, 'd, 'e, 'f, 'g, 'h>(_session: &'b Session, _browser: &'c dyn BrowserBridge, _account: &'d AccountId, _project: &'e ProjectId, _payload: Value, _cfg: &'f ChainConfig, _chat: &'g str, _events: Option<&'h (dyn Fn(ChainEvent) + Send + Sync)>) -> Result<AssembledResponse, String>);
    denied!("client", native_long<'b, 'c, 'd, 'e, 'f, 'g, 'h>(_session: &'b Session, _browser: &'c dyn BrowserBridge, _account: &'d AccountId, _project: &'e ProjectId, _payload: Value, _cfg: &'f ChainConfig, _chat: &'g str, _events: Option<&'h (dyn Fn(ChainEvent) + Send + Sync)>) -> Result<AssembledResponses, String>);
    denied!("client", claim_workspace_grant<'b>(_session: &'b Session) -> Result<bool, String>);
    denied!("client", create_project<'b, 'c>(_session: &'b Session, _team: &'c TeamId) -> Result<ProjectId, String>);
    denied!("client", team_projects<'b, 'c>(_session: &'b Session, _team: &'c TeamId) -> Result<Vec<ProjectId>, String>);
    denied!("client", create_workspace<'b, 'c>(_session: &'b Session, _spec: &'c WorkspaceSpec) -> Result<TeamId, String>);
}

impl SessionStore for FixturePorts {
    fn load<'s, 'b, 'a>(&'s self, _user: &'b FramerUserId)
        -> BoxFuture<'a, Result<Option<Session>, String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            let session = self.native_session.lock().unwrap().clone()
                .unwrap_or_else(|| self.unexpected("session"));
            self.session_calls.fetch_add(1, Ordering::SeqCst);
            Ok(session)
        })
    }
    denied!("session", delete<'b>(_user: &'b FramerUserId) -> Result<(), String>);

    fn save<'s, 'b, 'a>(&'s self, session: &'b Session) -> BoxFuture<'a, Result<(), String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            if !self.import { self.unexpected("session"); }
            self.session_calls.fetch_add(1, Ordering::SeqCst);
            self.saved.lock().unwrap().push(session.clone());
            Ok(())
        })
    }
}

impl StateStore for FixturePorts {
    fn get<'s, 'a>(&'s self, id: AccountId) -> BoxFuture<'a, Result<Option<ManagedAccount>, String>>
    where 's: 'a, Self: 'a {
        Box::pin(async move {
            self.state_reads.fetch_add(1, Ordering::SeqCst);
            Ok(self.records.lock().unwrap().iter().find(|a| a.id == id).cloned())
        })
    }

    fn list<'s, 'a>(&'s self) -> BoxFuture<'a, Result<Vec<ManagedAccount>, String>>
    where 's: 'a, Self: 'a {
        Box::pin(async move {
            self.state_reads.fetch_add(1, Ordering::SeqCst);
            Ok(self.records.lock().unwrap().clone())
        })
    }

    fn upsert<'s, 'b, 'a>(&'s self, account: &'b ManagedAccount) -> BoxFuture<'a, Result<(), String>>
    where 's: 'a, 'b: 'a, Self: 'a {
        Box::pin(async move {
            if !self.import { self.unexpected("state"); }
            self.state_writes.fetch_add(1, Ordering::SeqCst);
            let mut records = self.records.lock().unwrap();
            let record = records.iter_mut().find(|a| a.id == account.id).unwrap();
            *record = account.clone();
            Ok(())
        })
    }

    denied!("state", remove<>(_id: AccountId) -> Result<(), String>);
}

fn fixture(import: bool) -> (AppState, Arc<FixturePorts>) {
    let ports = Arc::new(FixturePorts {
        import,
        workspaces: Mutex::new(vec![("team-2", 100.0)]),
        records: Mutex::new(vec![account(1), account(2)]),
        saved: Mutex::new(Vec::new()),
        registered_proxies: Mutex::new(Vec::new()),
        browser_calls: AtomicUsize::new(0), client_calls: AtomicUsize::new(0),
        session_calls: AtomicUsize::new(0), state_reads: AtomicUsize::new(0),
        state_writes: AtomicUsize::new(0),
        native_session: Mutex::new(None),
        run_reply: Mutex::new(None),
        run_error: Mutex::new(None),
        run_calls: Mutex::new(Vec::new()),
    });
    let state = AppState::fixture(
        ports.clone(), ports.clone(), ports.clone(), ports.clone(),
        policy(&[1]), framer_core::gate::ConcurrencyGate::new(),
    );
    (state, ports)
}

async fn admit_workspaces(
    workspaces: Vec<(&'static str, f64)>, claimed: &[&str],
) -> (Result<TeamId, framer_core::error::CoreError>, Arc<FixturePorts>) {
    let (_, ports) = fixture(true);
    *ports.workspaces.lock().unwrap() = workspaces;
    let mut current = account(2);
    let session = Session { active_user: current.framer_user_id.clone(),
        cookie_header: "session=fixture-only".into() };
    let updates = tokio::sync::Mutex::new(());
    let deps = framer_core::pipeline::Deps {
        client: ports.as_ref(), browser: ports.as_ref(),
        sessions: ports.as_ref(), state: ports.as_ref(), account_updates: &updates,
    };
    let claimed = claimed.iter().map(|team| (*team).to_string()).collect();
    let result = framer_core::credits::admit(&deps, &mut current, &session, &claimed).await;
    assert_eq!(serde_json::to_value(&current).unwrap(),
        serde_json::to_value(&ports.records.lock().unwrap()[1]).unwrap());
    (result, ports)
}

#[tokio::test]
async fn an_unclaimed_workspace_is_accepted() {
    let (result, ports) = admit_workspaces(vec![("fresh", 500.0)], &[]).await;
    assert_eq!(result.unwrap().0, "fresh");
    let records = ports.records.lock().unwrap();
    assert_eq!(records[1].current_team, Some(TeamId("fresh".into())));
    assert_eq!(records[1].credit.unwrap().available, 500.0);
}

#[tokio::test]
async fn a_workspace_another_account_already_holds_is_refused() {
    let (result, ports) = admit_workspaces(vec![("shared", 500.0)], &["shared"]).await;
    assert!(matches!(result, Err(framer_core::error::CoreError::NotEligible(_))));
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_first_unclaimed_workspace_wins_even_if_poorer() {
    let (result, ports) = admit_workspaces(
        vec![("rich-but-taken", 900.0), ("poorer-free", 10.0)], &["rich-but-taken"],
    ).await;
    assert_eq!(result.unwrap().0, "poorer-free");
    let records = ports.records.lock().unwrap();
    assert_eq!(records[1].current_team, Some(TeamId("poorer-free".into())));
    assert_eq!(records[1].credit.unwrap().available, 10.0);
}

#[tokio::test]
async fn no_credited_workspace_at_all_is_refused() {
    let (result, ports) = admit_workspaces(Vec::new(), &[]).await;
    assert!(matches!(result, Err(framer_core::error::CoreError::NotEligible(_))));
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), 0);
}

fn parked_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-framerize-account", account(2).id.0.to_string().parse().unwrap());
    headers
}

#[tokio::test]
async fn non_waiting_routes_reject_all_cooling_before_external_io() {
    use axum::response::IntoResponse;
    for dialect in [ClientDialect::AnthropicMessages, ClientDialect::OpenAiResponses, ClientDialect::OpenAiChat] {
        for stream in [false, true] {
            // Waiting streams have separate ping, disconnect and budget checks.
            if stream && dialect != ClientDialect::OpenAiChat { continue; }
            let (mut state, ports) = fixture(false);
            state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
            let now = framer_core::rotation::now_ms();
            ports.records.lock().unwrap()[0].cooling_until_ms = Some(now + 600_000);
            ports.records.lock().unwrap()[1].cooling_until_ms = Some(now + 300_000);
            let before = serde_json::to_value(&*ports.records.lock().unwrap()).unwrap();
            let payload = json!({"model": "gpt-6-astra", "stream": stream, "input": "fixture",
                "messages": [{"role": "user", "content": "fixture"}]});
            let response = crate::free_complete(state.clone(), &HeaderMap::new(), dialect, payload)
                .await.unwrap_err().into_response();
            assert_eq!(response.status(), 503);
            assert_eq!(response.headers()["retry-after"], "30");
            assert_eq!(response.headers()["content-type"], "application/json");
            let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["error"]["code"], "framerize_account_cooling");
            assert_eq!(body["error"]["retry_after_seconds"], 30);
            assert!((299..=300).contains(&body["error"]["cooldown_remaining_seconds"].as_u64().unwrap()));
            assert_eq!(before, serde_json::to_value(&*ports.records.lock().unwrap()).unwrap());
            assert!(state.wave.check().is_none());
            ports.assert_rejected_before_io(1);
        }
    }
}

#[tokio::test]
async fn wave_cooldown_returns_retry_after_and_json_before_external_io() {
    use axum::response::IntoResponse;
    for (dialect, stream) in [(ClientDialect::AnthropicMessages, false),
        (ClientDialect::OpenAiResponses, false)] {
        let (state, ports) = fixture(false);
        state.wave.note_refusal("fixture-a");
        state.wave.note_refusal("fixture-b");
        let payload = json!({
            "model": "gpt-6-astra", "stream": stream,
            "input": "fixture", "messages": [{"role": "user", "content": "fixture"}]
        });
        let error = crate::free_complete(state.clone(), &HeaderMap::new(), dialect, payload).await.unwrap_err();
        let response = error.into_response();
        assert_eq!(response.status(), 503);
        assert_eq!(response.headers()["retry-after"], "30");
        assert!(state.wave.check().unwrap() > std::time::Duration::from_secs(30));
        assert_eq!(response.headers()["content-type"], "application/json");
        let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], "framerize_cooling");
        assert_eq!(body["error"]["retry_after_seconds"], 30);
        assert!(body["error"]["cooldown_remaining_seconds"].as_u64().unwrap() > 30);
        ports.assert_rejected_before_io(0);
    }
}

#[tokio::test]
async fn explicit_cooling_and_retry_selection_stop_before_io() {
    let (mut state, ports) = fixture(false);
    state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
    ports.records.lock().unwrap()[0].cooling_until_ms = Some(framer_core::rotation::now_ms() + 600_000);
    let mut headers = HeaderMap::new();
    headers.insert("x-framerize-account", account(1).id.0.to_string().parse().unwrap());
    let error = crate::free_complete(state.clone(), &headers, ClientDialect::OpenAiResponses,
        json!({"model": "gpt-6-astra", "input": "fixture", "stream": true}))
        .await.unwrap_err();
    assert!(matches!(error, ApiError::AccountCooling { .. }));
    let error = crate::pick_fresh_account(&state, &[account(2).id.0.to_string()]).await.result.err().expect("expected rejection");
    assert!(matches!(error, ApiError::AccountCooling { .. }));
    ports.assert_rejected_before_io(2);
}

#[tokio::test]
async fn compat_cooling_stops_before_opening_a_stream_or_loading_a_session() {
    for stream in [false, true] {
        let (state, ports) = fixture(false);
        ports.records.lock().unwrap()[0].cooling_until_ms = Some(framer_core::rotation::now_ms() + 600_000);
        let error = crate::compat_forward(state, "openai-responses", None, None,
            json!({"model": "fixture", "input": "fixture", "stream": stream}))
            .await.unwrap_err();
        assert!(matches!(error, ApiError::AccountCooling { .. }));
        ports.assert_rejected_before_io(1);
    }
}

#[tokio::test]
async fn cooldown_guidance_bounds_local_rechecks_and_preserves_full_pause() {
    use axum::response::IntoResponse;
    for (remaining, recheck) in [
        (0, 1), (1, 1), (29, 29), (30, 30), (31, 30), (150, 30),
        (900, 30), (u64::MAX, 30),
    ] {
        for (error, code) in [
            (ApiError::Cooling { retry_after_secs: remaining }, "framerize_cooling"),
            (ApiError::AccountCooling { retry_after_secs: remaining,
                cause: "fixture request allowance exhausted".into() }, "framerize_account_cooling"),
        ] {
            let response = error.into_response();
            assert_eq!(response.status(), 503);
            assert_eq!(response.headers()["retry-after"].to_str().unwrap(), recheck.to_string());
            assert_eq!(response.headers()["content-type"], "application/json");
            let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["type"], "error");
            assert_eq!(body["error"]["type"], "overloaded_error");
            assert_eq!(body["error"]["code"], code);
            assert_eq!(body["error"]["retry_after_seconds"], recheck);
            assert_eq!(body["error"]["cooldown_remaining_seconds"], remaining);
            assert!(body["error"]["message"].as_str().unwrap()
                .contains(&format!("Retry in {recheck} seconds.")));
        }
    }
}

#[tokio::test]
async fn local_rechecks_keep_wave_closed_until_an_actual_success() {
    for dialect in [
        ClientDialect::AnthropicMessages, ClientDialect::OpenAiResponses,
        ClientDialect::OpenAiChat,
    ] {
        let (state, ports) = fixture(false);
        for _ in 0..4 {
            state.wave.note_refusal("fixture-a");
            state.wave.note_refusal("fixture-b");
        }
        let payload = json!({
            "model": "gpt-6-astra", "stream": false, "input": "fixture",
            "messages": [{"role": "user", "content": "fixture"}]
        });
        for _ in 0..3 {
            let result = crate::free_complete(
                state.clone(), &parked_headers(), dialect, payload.clone(),
            ).await;
            assert!(matches!(result, Err(ApiError::Cooling { retry_after_secs }) if retry_after_secs > 800));
            assert!(state.wave.check().unwrap() > std::time::Duration::from_secs(800));
            ports.assert_rejected_before_io(0);
        }
        state.wave.note_success();
        assert!(state.wave.check().is_none());
        let result = crate::free_complete(state, &parked_headers(), dialect, payload).await;
        assert!(matches!(result, Err(ApiError::ServingCapacity(_))));
        ports.assert_rejected_before_io(1);
    }
}

#[tokio::test]
async fn real_refusals_still_pause_the_pool_and_local_cooldowns_do_not_count() {
    let (state, ports) = fixture(true);
    *ports.records.lock().unwrap() = (1..=3).map(account).collect();
    crate::record_account_health(&state, &account(1), &Err::<(), _>("risk-refused".into())).await;
    crate::record_account_health(&state, &account(2), &Err::<(), _>("account-cooling:584".into())).await;
    assert!(state.wave.check().is_none());
    crate::record_account_health(&state, &account(3), &Err::<(), _>("risk-refused".into())).await;
    assert!(state.wave.check().is_some());
    let records = ports.records.lock().unwrap();
    assert_eq!(records[0].consecutive_failures, 1);
    assert_eq!(records[1].consecutive_failures, 0);
    assert_eq!(records[2].consecutive_failures, 1);
}

#[tokio::test]
async fn old_success_cannot_clear_a_newly_recorded_cooldown_cause() {
    let (state, ports) = fixture(true);
    let mut before = account(1);
    before.note_cooldown(u64::MAX, None);
    let mut current = before.clone();
    current.note_cooldown(u64::MAX, Some("device allowance exhausted"));
    *ports.records.lock().unwrap() = vec![current];
    crate::record_account_health(&state, &before, &Ok::<_, String>(())).await;
    let records = ports.records.lock().unwrap();
    assert_eq!(records[0].cooling_until_ms, Some(u64::MAX));
    assert_eq!(records[0].cooldown_reason(), "device allowance exhausted");
}

#[tokio::test]
async fn local_cooldowns_park_accounts_without_pausing_the_serving_pool() {
    let (mut state, ports) = fixture(true);
    *ports.records.lock().unwrap() = (1..=4).map(account).collect();
    state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2, 3]));
    let before = framer_core::rotation::now_ms();
    for (id, error) in [(1, "account-cooling:584; device allowance exhausted"),
        (2, "chunk 0 retry died (fallback off): account-cooling:584; device allowance exhausted")] {
        crate::record_account_health(&state, &account(id), &Err::<(), _>(error.to_string())).await;
    }
    assert!(state.wave.check().is_none());
    let records = ports.records.lock().unwrap().clone();
    for record in &records[..2] {
        assert!(record.cooling_until_ms.unwrap() >= before + 584_000);
        assert_eq!(record.consecutive_failures, 0);
        assert_eq!(record.cooldown_step, 0);
        assert_eq!(record.cooldown_reason(), "device allowance exhausted");
    }
    assert_eq!(state.serving_accounts.select(records, None).unwrap().id, account(3).id);
    assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.client_calls.load(Ordering::SeqCst), 0);
}

fn assert_capacity<T>(result: Result<T, ApiError>, ports: &FixturePorts, reads: usize) {
    assert!(matches!(result, Err(ApiError::ServingCapacity(_))));
    ports.assert_rejected_before_io(reads);
}

#[tokio::test]
async fn parked_run_stops_before_state_session_browser_or_client_io() {
    let (state, ports) = fixture(false);
    let result = crate::run(State(state), Json(crate::RunBody {
        account_id: account(2).id, prompt: "fixture".into(), model: None, chat_session: None,
    })).await;
    assert_capacity(result, &ports, 0);
}

#[tokio::test]
async fn parked_login_begin_stops_before_all_io() {
    let (state, ports) = fixture(false);
    let result = crate::begin_login(State(state), Json(crate::BeginLoginBody {
        account_id: account(2).id, email: "fixture@example.invalid".into(),
    })).await;
    assert_capacity(result, &ports, 0);
}

#[tokio::test]
async fn parked_login_complete_stops_before_all_io() {
    let (state, ports) = fixture(false);
    let result = crate::complete_login(State(state), Json(crate::CompleteLoginBody {
        account_id: account(2).id, email: None, email_token: "fixture-only".into(),
        alias: None, project_id: None,
    })).await;
    assert_capacity(result, &ports, 0);
}

#[tokio::test]
async fn parked_compat_forward_stops_before_session_browser_or_client_io() {
    for stream in [false, true] {
        let (state, ports) = fixture(false);
        let result = crate::compat_forward(
            state, "openai-responses", Some(account(2).id.0.to_string()), None,
            json!({"model": "fixture", "input": "fixture", "stream": stream}),
        ).await;
        assert_capacity(result, &ports, 1);
    }
}

#[tokio::test]
async fn parked_free_complete_stops_before_session_browser_or_client_io() {
    for stream in [false, true] {
        let (state, ports) = fixture(false);
        let result = crate::free_complete(
            state, &parked_headers(), ClientDialect::AnthropicMessages,
            json!({"model": "claude-opus-5-5", "stream": stream,
                "messages": [{"role": "user", "content": "fixture"}]}),
        ).await;
        assert_capacity(result, &ports, 1);
    }
}

#[tokio::test]
async fn parked_native_complete_stops_before_session_browser_or_client_io() {
    for stream in [false, true] {
        let (state, ports) = fixture(false);
        let result = crate::native_complete(
            state, &parked_headers(), ClientDialect::OpenAiResponses,
            json!({"model": "gpt-6-astra", "input": "fixture", "stream": stream}),
            stream, false, Some(64),
        ).await;
        assert_capacity(result, &ports, 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn parked_import_preserves_session_and_proxy_without_browser_work() {
    let (state, ports) = fixture(true);
    let first_before = serde_json::to_value(&ports.records.lock().unwrap()[0]).unwrap();
    let proxy = "http://replacement-fixture.invalid:8080";
    let result = crate::import_account(State(state), Json(crate::ImportBody {
        account_id: account(2).id,
        expected_framer_user_id: None,
        proxy: Some(proxy.into()),
        session_cookie: "fixture-only".into(),
        email: None, alias: None, project_id: None,
        team_id: Some("team-2".into()),
    })).await.unwrap();
    assert_eq!(result.0["status"], "imported");
    assert_eq!(result.0["serving_allowed"], false);
    assert_eq!(*ports.registered_proxies.lock().unwrap(), vec![(account(2).id, Some(proxy.into()))]);
    assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
    // A prewarm task would own an extra browser Arc before its first poll.
    // This current-thread runtime cannot run that task between the handler's
    // non-awaiting final spawn and this check. No timing or sleep is needed.
    assert_eq!(Arc::strong_count(&ports), 1, "a background task retained browser access");
    assert!(ports.client_calls.load(Ordering::SeqCst) > 0);
    assert_eq!(ports.session_calls.load(Ordering::SeqCst), 1);
    let saved = ports.saved.lock().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].active_user.0, "user-2");
    assert_eq!(saved[0].cookie_header, "session=fixture-only");
    let records = ports.records.lock().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(serde_json::to_value(&records[0]).unwrap(), first_before);
    assert_eq!(records[1].id, account(2).id);
    assert_eq!(records[1].castle_device_id, account(2).castle_device_id);
    assert_eq!(records[1].proxy.as_deref(), Some(proxy));
    assert_eq!(records[1].current_project, account(2).current_project);
}

#[tokio::test(flavor = "current_thread")]
async fn import_succeeds_when_optional_prewarm_registration_has_no_capacity() {
    use tower::ServiceExt;
    use axum::{body::{Body, to_bytes}, http::Request};
    let (mut state, ports) = fixture(true);
    state.runtime_mut().expect("unique fixture runtime").serving_accounts = Arc::new(policy(&[1, 2]));
    // Exactly host + whole-handler ownership fit; optional child cannot fit.
    state = crate::operation_ownership::with_owned_operations(state, 2).unwrap();
    let before = serde_json::to_value(&ports.records.lock().unwrap()[0]).unwrap();
    let request = Request::builder().method("POST").uri("/api/accounts/import")
        .header("content-type", "application/json")
        .body(Body::from(json!({"account_id":account(2).id,
            "session_cookie":"fixture-only","team_id":"team-2"}).to_string())).unwrap();
    let response = tokio::time::timeout(std::time::Duration::from_secs(3),
        crate::router(state, "/nonexistent-fixture-web").oneshot(request)).await.unwrap().unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(body["status"], "imported");
    assert_eq!(body["serving_allowed"], true);
    assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
    assert_eq!(*ports.registered_proxies.lock().unwrap(), vec![(account(2).id, account(2).proxy)]);
    assert_eq!(ports.session_calls.load(Ordering::SeqCst), 1);
    assert!(ports.state_writes.load(Ordering::SeqCst) > 0);
    let saved = ports.saved.lock().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].active_user.0, "user-2");
    let records = ports.records.lock().unwrap();
    assert_eq!(serde_json::to_value(&records[0]).unwrap(), before);
    assert_eq!(records[1].framer_user_id.0, "user-2");
    assert_eq!(records[1].current_project, account(2).current_project);
}
