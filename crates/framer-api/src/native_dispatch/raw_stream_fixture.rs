// Only included by the raw-stream test module. All identities are synthetic.
include!("fixture.rs");
use framer_core::native_dispatch::{RawStreamBody, RawStreamCompletion,
    RawStreamFinished, RawStreamLimits, RawStreamRun};
use crate::experimental_native::{application::ApplicationError,
    native_owner::{AdmissionError, Inputs, RequestPool}};

const PAUSE: Duration = Duration::from_millis(25);

struct ForeignBacking { bytes: Box<[u8]>, dropped: Arc<AtomicUsize> }
impl AsRef<[u8]> for ForeignBacking { fn as_ref(&self) -> &[u8] { &self.bytes } }
impl Drop for ForeignBacking {
    fn drop(&mut self) { self.dropped.fetch_add(1, Ordering::Release); }
}

fn foreign_stream(f: &Fixture, visible: usize, dropped: Arc<AtomicUsize>) -> RawStreamRun {
    super::raw_stream::launch_for_test(call_for_hook(f, deadline()), stream_payload(), limits(),
        Box::new(move |parts, payload| {
            let mut pending = pending_with_hooks(parts, payload, TestHooks::default());
            let response = pending.response;
            pending.response = Box::pin(async move {
                let mut response = response.await?;
                let large = bytes::Bytes::from_owner(ForeignBacking {
                    bytes: vec![42; 1024 * 1024].into_boxed_slice(), dropped,
                });
                response.proxy.stream = Box::pin(futures::stream::iter(vec![Ok(large.slice(..visible))]));
                Ok(response)
            });
            Ok(pending)
        }))
}

fn limits() -> RawStreamLimits {
    RawStreamLimits { chunk_bytes: 5, retained_bytes: 10, total_bytes: 16 * 1024,
        header_bytes: 1024 }
}

fn stream_payload() -> Value {
    let mut payload = responses_payload(); payload["stream"] = json!(true); payload
}

async fn start(f: &Fixture, until: Instant, limits: RawStreamLimits) -> RawStreamRun {
    f.admit(Route::Raw(ClientDialect::OpenAiResponses), until).await.unwrap()
        .raw_stream(stream_payload(), limits)
}

async fn drain(mut body: RawStreamBody) -> Vec<u8> {
    tokio::time::timeout(STEP, async move {
        let mut bytes = Vec::new();
        while let Some(chunk) = body.next().await { bytes.extend_from_slice(&chunk.unwrap()); }
        bytes
    }).await.unwrap()
}

async fn finished(value: RawStreamFinished) -> RawStreamCompletion {
    tokio::time::timeout(STEP, value).await.unwrap()
}

async fn cleanup(value: port::Cleanup) {
    assert_eq!(tokio::time::timeout(STEP, value).await.unwrap(), Ok(()));
}

fn call_for_hook(f: &Fixture, until: Instant) -> Box<super::NativeCall> {
    Box::new(super::NativeCall { api: f.provider.api.clone(), native: f.client.clone(),
        scope: f.scope.clone(), authorization: f.supplier.clone(),
        account_lease: super::shared_lease::AccountLease::Unmanaged,
        request: f.request(Route::Raw(ClientDialect::OpenAiResponses), until),
        policy: policy(), permit: f.capacity.clone().try_acquire_owned().unwrap() })
}

fn pending_with_start(parts: &super::Parts, payload: Value,
    start: impl FnOnce(&RequestPool, Inputs, Instant, String)
        -> Result<bridge::Pending, bridge::StartError>) -> Result<bridge::Pending, ClientError> {
    let context = RequestContext {
        dialect: Dialect::OpenAiResponses, project: project(), payload,
        feature_category: "fixture-application-category".into(),
        feature_name: "fixture-application-feature".into(), step: "fixture-application-step".into(),
        chat_session: CHAT.into(), step_index: 0, turn_id: "fixture-application-turn-0".into(),
        user_prompt: "fixture outer annotation\nkept exactly".into(),
    };
    parts.native.start_for_test(&parts.scope, context, auth(&parts.scope),
        parts.request.deadline, MISSING_TYPE.into(), start)
}

fn pending_with_hooks(parts: &super::Parts, payload: Value, hooks: TestHooks) -> bridge::Pending {
    pending_with_start(parts, payload,
        |pool, inputs, until, content_type| {
            let owner = pool.start_for_test(inputs, Mode::Raw, until, hooks)
                .map_err(bridge::StartError::Admission)?;
            bridge::from_owner(owner, content_type).map_err(bridge::StartError::Attach)
        }).unwrap()
}

fn hooked_stream(f: &Fixture, until: Instant, hooks: TestHooks) -> RawStreamRun {
    super::raw_stream::launch_for_test(call_for_hook(f, until), stream_payload(), limits(),
        Box::new(move |parts, payload| Ok(pending_with_hooks(parts, payload, hooks))))
}
