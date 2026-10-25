// Dispatch fixture support adapted from accepted application helpers. Source only.
// Included only under cfg(test). All request/auth/response values are synthetic.

use crate::request::invocation::AttemptAuthorization;

use crate::request::invocation::AttemptContext;

use crate::request::invocation::AuthorizedAttempt;

use crate::request::invocation::PreparedAttempt;
use crate::account::client::AccountScope;
use crate::account::client::ClientError;
use crate::account::client::NativeAccountClient;
use crate::account::client::SuppliedAuth;
use crate::transport::http::FreshCastleToken;
use crate::transport::http::NativeHttp;
use crate::transport::http::NativeHttpLimits;
use crate::account::owner::EndReason;
use crate::account::owner::StopReason;
use framer_api::proxy::Dialect;
use framer_core::ids::AccountId;
use framer_core::ids::FramerUserId;
use framer_core::ids::ProjectId;
use framer_core::port::CastleToken;
use framer_core::port::Session;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use futures::Future;
use futures::StreamExt;
use serde_json::json;
use serde_json::Value;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

#[path = "loopback.rs"]
mod loopback;
use loopback::Expected;
use loopback::Reply;
use loopback::Server;
use loopback::Snapshot;
use loopback::STEP;
use loopback::TICK;

const CHAT: &str = "fixture-application-chat";
const MISSING_TYPE: &str = "application/x-explicit-fixture";
const CALLER_CONTINUE: &str = "Fixture caller says: continue\nexactly this supplied text.";

fn responses_payload() -> Value {
    json!({
        "model": "gpt-4.1", "stream": false, "max_output_tokens": 999,
        "input": [
            {"type": "message", "role": "developer", "content": [
                {"type": "input_text", "text": "fixture developer\nhistory"}]},
            {"type": "function_call", "call_id": "fixture-call", "name": "fixture_tool", "namespace": "fixture_ns",
             "arguments": "{\"value\":\"keep\\\\exact\"}", "status": "completed"},
            {"type": "function_call_output", "call_id": "fixture-call", "output": [
                {"type": "input_text", "text": "fixture tool result"}]}
        ],
        "tools": [{"type": "namespace", "name": "fixture_ns", "tools": [
            {"type": "function", "name": "fixture_tool", "strict": true,
             "parameters": {"type": "object", "properties": {"value": {"type": "string"}},
                            "additionalProperties": false, "required": ["value"]}}
        ]}],
        "parallel_tool_calls": false, "tool_choice": "auto",
        "reasoning": {"effort": "max", "summary": "auto"},
        "service_tier": "priority", "metadata": {"fixture": "line one\nline two"},
        "fixture_extension": {"opaque": [null, 3, "unchanged"]}
    })
}

fn expected_responses() -> Value {
    // Independent first-attempt oracle: the configured total cap is 32.
    let mut value = responses_payload();
    value["model"] = json!("openai/gpt-4.1");
    value["stream"] = json!(false);
    value["max_output_tokens"] = json!(32);
    value["input"][1]["namespace"] = json!("fixture_ns");
    for key in ["store", "previous_response_id", "background"] {
        value.as_object_mut().expect("fixture object").remove(key);
    }
    value
}

fn messages_payload() -> Value {
    json!({
        "model": "anthropic/fixture-model", "stream": false, "max_tokens": 999,
        "system": [{"type": "text", "text": "fixture system\nhistory",
                    "cache_control": {"type": "ephemeral"}}],
        "messages": [
            {"role": "user", "content": [{"type": "text", "text": "fixture request"}]},
            {"role": "assistant", "content": [{"type": "tool_use", "id": "fixture-tool",
                "name": "fixture_tool", "input": {"value": "keep\nexact"}}]},
            {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "fixture-tool",
                "content": [{"type": "text", "text": "fixture tool result"}]}]}
        ],
        "tools": [{"name": "fixture_tool", "description": "fixture description",
                   "input_schema": {"type": "object", "properties": {"value": {"type": "string"}}}}],
        "tool_choice": {"type": "auto"}, "thinking": {"type": "enabled", "budget_tokens": 1024},
        "metadata": {"user_id": "fixture-user"}, "stop_sequences": ["fixture stop"],
        "temperature": 0.25, "fixture_extension": {"opaque": [null, false, "unchanged"]}
    })
}

fn expected_messages() -> Value {
    let mut value = messages_payload();
    value["stream"] = json!(false);
    value["max_tokens"] = json!(32);
    value
}

fn message_body() -> Value {
    json!({
        "id": "fixture-message", "content": [
            {"type": "text", "text": "fixture result", "fixture_detail": {"opaque": true}}
        ], "stop_reason": "end_turn", "stop_sequence": "fixture stop",
        "usage": {"input_tokens": 11, "output_tokens": 7, "cache_read_input_tokens": 3,
                  "fixture_detail": [1, 2, 3]}
    })
}

fn response_body() -> Value {
    json!({
        "id": "fixture-response", "created_at": 1234567890, "status": "completed",
        "output": [
            {"type": "reasoning", "id": "fixture-reasoning",
             "summary": [{"type": "summary_text", "text": "fixture summary"}],
             "encrypted_content": "fixture-opaque"},
            {"type": "function_call", "id": "fixture-output-call", "call_id": "fixture-next",
             "name": "fixture_tool", "namespace": "fixture_ns", "status": "completed",
             "arguments": "{\"value\":\"fixture result\"}", "fixture_detail": [1, 2]}
        ],
        "usage": {"input_tokens": 11, "output_tokens": 7, "total_tokens": 18,
                  "output_tokens_details": {"reasoning_tokens": 2},
                  "input_tokens_details": {"cached_tokens": 3}},
        "service_tier": "priority", "incomplete_details": null
    })
}

fn expected(path: &'static str, payload: Value, index: u32, reply: Reply) -> Expected {
    let mut expected = Expected {
        path,
        envelope: json!({
            "projectId": "fixture-application-project",
            "featureCategory": "fixture-application-category",
            "featureName": "fixture-application-feature",
            "step": "fixture-application-step", "sessionId": CHAT, "stepIndex": index,
            "turnId": format!("fixture-application-turn-{index}"),
            "userPrompt": "fixture outer annotation\nkept exactly",
            "model": payload["model"], "request": payload,
            "castleToken": "fixture-application-castle"
        }),
        reply,
    };
    if path == "/ai/v3/responses/" {
        expected.envelope.as_object_mut().unwrap().remove("model");
        expected.envelope["request"]["session_id"] = json!(CHAT);
    }
    expected
}

fn metadata(value: &AttemptMetadata, index: u32) {
    assert_eq!(value.status, 200);
    assert_eq!(value.usage_header.as_deref(), Some("remaining=7.5&fixture=application"));
    assert_eq!(value.original_content_type.as_deref(), Some("application/json"));
    assert_eq!(value.step_index, index);
}

fn messages_continuation() -> (Value, Value) {
    let mut first = message_body();
    first["stop_reason"] = json!("max_tokens");
    let mut next = expected_messages();
    next["max_tokens"] = json!(25);
    next["messages"].as_array_mut().unwrap().extend([
        json!({"role": "assistant", "content": first["content"]}),
        json!({"role": "user", "content": CALLER_CONTINUE}),
    ]);
    (first, next)
}

fn responses_continuation() -> (Value, Value) {
    let mut first = response_body();
    first["status"] = json!("incomplete");
    first["incomplete_details"] = json!({"reason": "max_output_tokens"});
    first["output"][1] = json!({
        "type": "message", "id": "fixture-text-item", "role": "assistant",
        "status": "incomplete", "content": [
            {"type": "output_text", "text": "fixture first text", "annotations": []}
        ], "fixture_detail": {"opaque": true}
    });
    let mut next = expected_responses();
    next["max_output_tokens"] = json!(25);
    next["input"].as_array_mut().unwrap()
        .extend(first["output"].as_array().unwrap().clone());
    next["input"].as_array_mut().unwrap().push(json!({
        "type": "message", "role": "user", "content": [
            {"type": "input_text", "text": CALLER_CONTINUE}
        ]
    }));
    (first, next)
}

struct RuntimeOwner(Option<tokio::runtime::Runtime>);

impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() { runtime.shutdown_timeout(STEP); }
    }
}

fn run<F: Future>(work: F) -> F::Output {
    let runtime = RuntimeOwner(Some(tokio::runtime::Builder::new_current_thread()
        .enable_all().max_blocking_threads(1).build().expect("application fixture runtime")));
    runtime.0.as_ref().expect("owned runtime").block_on(async {
        tokio::time::timeout(Duration::from_secs(8), work).await
            .expect("application fixture deadline")
    })
}

fn project() -> ProjectId { ProjectId("fixture-application-project".into()) }
fn deadline() -> Instant { Instant::now() + Duration::from_secs(3) }

async fn wait_count(count: &AtomicUsize, expected: usize) {
    tokio::time::timeout(STEP, async {
        while count.load(Ordering::Acquire) < expected { tokio::time::sleep(TICK).await; }
    }).await.expect("application phase deadline");
}

struct Identity { account: AccountId, session: Session }

impl Identity {
    fn new() -> Self {
        Self {
            account: AccountId::new(),
            session: Session {
                active_user: FramerUserId("fixture-application-user".into()),
                cookie_header: "session=fixture-application".into(),
            },
        }
    }

    fn open(&self, client: &NativeAccountClient, server: &Server) -> AccountScope {
        client.open_account_for_test(self.account, self.session.clone(), || {
            NativeHttp::for_loopback(self.account, self.session.active_user.clone(), server.address,
                NativeHttpLimits {
                    request_bytes: 32 * 1024, buffered_response_bytes: 16 * 1024,
                    streamed_response_bytes: 32 * 1024, incoming_chunk_bytes: 16 * 1024,
                    emitted_chunk_bytes: 5, credential_bytes: 256, in_flight: 1,
                    connect_timeout: Duration::from_secs(1),
                    request_timeout: Duration::from_secs(5),
                    token_max_age: Duration::from_secs(5),
                })
        }).expect("application fixture account")
    }
}

async fn close(client: &NativeAccountClient, scope: &AccountScope) {
    let closed = client.close_account(scope).expect("application close admission");
    assert_eq!(tokio::time::timeout(STEP, closed).await.expect("application close deadline"), Ok(()));
}

async fn retire(client: NativeAccountClient) {
    let supervisor = client.supervisor_for_test();
    drop(client);
    tokio::time::timeout(STEP, async {
        while !supervisor.is_finished() { tokio::time::sleep(TICK).await; }
    }).await.expect("application supervisor exit deadline");
}

fn checked(snapshot: &Snapshot, requests: usize, replies: usize) {
    assert_eq!(snapshot.fault, None);
    assert_eq!(snapshot.accepted, requests);
    assert_eq!(snapshot.requests, requests);
    assert_eq!(snapshot.headers, requests);
    assert_eq!(snapshot.replies, replies);
}

fn auth(scope: &AccountScope) -> SuppliedAuth {
    SuppliedAuth::new(scope, "fixture-application-bearer".into(),
        FreshCastleToken::from_sdk_output(scope.account(),
            CastleToken("fixture-application-castle".into())))
}

struct Seen {
    account: AccountId,
    generation: u64,
    project: String,
    dialect: String,
    payload: Value,
    chat: String,
    index: u32,
    deadline: Instant,
}

#[derive(Default)]
struct Supplier {
    calls: AtomicUsize,
    cancelled: AtomicUsize,
    seen: Mutex<Vec<Seen>>,
    gate: Mutex<Option<oneshot::Receiver<()>>>,
    gate_index: u32,
    held_auth: Mutex<Option<SuppliedAuth>>,
    never: bool,
    missing_context: bool,
}

impl Supplier {
    fn held() -> (Self, oneshot::Sender<()>) {
        let (release, gate) = oneshot::channel();
        (Self { gate: Mutex::new(Some(gate)), ..Self::default() }, release)
    }
}

struct SupplyGuard<'a> { cancelled: &'a AtomicUsize, complete: bool }
impl Drop for SupplyGuard<'_> {
    fn drop(&mut self) {
        if !self.complete { self.cancelled.fetch_add(1, Ordering::Release); }
    }
}

impl AttemptAuthorization for Supplier {
    fn authorize<'a>(
        &'a self, attempt: PreparedAttempt<'a>,
    ) -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        Box::pin(async move {
            let mut guard = SupplyGuard { cancelled: &self.cancelled, complete: false };
            {
                let mut seen = self.seen.lock().expect("supplier trace");
                assert!(seen.len() < 4, "unexpected extra authorization attempts");
                seen.push(Seen {
                    account: attempt.scope.account(), generation: attempt.scope.generation(),
                    project: attempt.project.0.clone(), dialect: format!("{:?}", attempt.dialect),
                    payload: attempt.payload.clone(), chat: attempt.chat_session.to_owned(),
                    index: attempt.step_index, deadline: attempt.deadline,
                });
            }
            self.calls.fetch_add(1, Ordering::Release);
            if self.never && attempt.step_index == self.gate_index {
                futures::future::pending::<()>().await;
            }
            let gate = if attempt.step_index == self.gate_index {
                self.gate.lock().expect("supplier gate").take()
            } else { None };
            if let Some(gate) = gate {
                gate.await.map_err(|_| "synthetic supplier was abandoned".to_owned())?;
            }
            let supplied = self.held_auth.lock().expect("held synthetic auth").take()
                .unwrap_or_else(|| auth(attempt.scope));
            guard.complete = true;
            Ok(AuthorizedAttempt {
                auth: supplied,
                context: AttemptContext {
                    feature_category: "fixture-application-category".into(),
                    feature_name: "fixture-application-feature".into(),
                    step: "fixture-application-step".into(),
                    turn_id: if self.missing_context { String::new() }
                        else { format!("fixture-application-turn-{}", attempt.step_index) },
                    user_prompt: "fixture outer annotation\nkept exactly".into(),
                },
            })
        })
    }
}

fn seen(
    supplier: &Supplier, scope: &AccountScope, until: Instant,
    dialect: &str, payloads: &[Value],
) {
    let calls = supplier.seen.lock().expect("supplier trace");
    assert_eq!(supplier.calls.load(Ordering::Acquire), payloads.len());
    assert_eq!(calls.len(), payloads.len());
    for (index, (call, payload)) in calls.iter().zip(payloads).enumerate() {
        assert_eq!(call.account, scope.account());
        assert_eq!(call.generation, scope.generation());
        assert_eq!(call.project, project().0);
        assert_eq!(call.dialect, dialect);
        assert_eq!(&call.payload, payload);
        assert_eq!(call.chat, CHAT);
        assert_eq!(call.index, index as u32);
        assert_eq!(call.deadline, until);
    }
}

use crate::request::types::{self as port, AttemptMetadata, DispatchError, FailureKind, InvocationRun};
use framer_core::{ids::TeamId, operation_gate::{OperationGate, OperationKind}, resident_pool::{Acquire, ResidentPool}};
use futures::FutureExt;
use tokio::sync::Semaphore;

struct Fixture {
    client: NativeAccountClient, scope: AccountScope, supplier: Arc<Supplier>,
    capacity: Arc<Semaphore>, pool: ResidentPool, gate: OperationGate,
}
impl Fixture {
    fn identity(&self) -> port::Identity {
        port::Identity { account: self.scope.account(), user: self.scope.active_user().clone(),
            team: TeamId("fixture-team".into()), project: project(), chat_session: CHAT.into() }
    }
    fn admission(&self) -> port::PoolAdmission {
        let Acquire::Lease(lease) = self.pool.try_acquire(self.scope.account(), None).unwrap()
            else { panic!("fixture pool capacity"); };
        port::PoolAdmission::new(lease, Some(self.gate.admit(OperationKind::Inference).unwrap()))
    }
    async fn finish(self) {
        match self.scope.closed().now_or_never() {
            Some(result) => assert_eq!(result, Ok(())),
            None => close(&self.client, &self.scope).await,
        }
        assert_eq!(self.capacity.available_permits(), 1);
        assert_eq!(self.gate.stats().unwrap().active, 0);
        assert!(!self.gate.stats().unwrap().unconfirmed);
        for resident in self.pool.snapshot().unwrap().residents {
            assert_eq!(resident.users, 0); assert!(!resident.uncertain);
        }
        retire(self.client).await;
    }
    fn new(server: &Server, supplier: Supplier) -> Self {
        let client = NativeAccountClient::new().unwrap();
        let scope = Identity::new().open(&client, server);
        Self { client, scope, supplier: Arc::new(supplier), capacity: Arc::new(Semaphore::new(1)),
            pool: ResidentPool::new(2).unwrap(), gate: OperationGate::new(8).unwrap() }
    }
}
