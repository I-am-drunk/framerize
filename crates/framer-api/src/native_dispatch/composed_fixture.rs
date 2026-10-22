// Synthetic fixture support only. No live account or supplier access.
include!("raw_stream_fixture.rs");
use framer_core::native_dispatch::{ComposedStreamOptions, ComposedStreamLimits,
    ComposedStreamRun, ComposedStreamCompletion};

fn options(dialect: ClientDialect) -> ComposedStreamOptions {
    ComposedStreamOptions { dialect, include_usage: true, limits: ComposedStreamLimits {
        frame_bytes: 8192, retained_bytes: 16384,
        retained_frames: if dialect == ClientDialect::OpenAiChat { 3 } else { 2 },
        state_bytes: 64 * 1024 * 1024, json_bytes: 16 * 1024,
        source_chunk_bytes: 5, header_bytes: 4096, total_bytes: 1024 * 1024,
        terminal_bytes: 1024,
    } }
}

async fn compose(f: &Fixture, route: Route, payload: Value,
    options: ComposedStreamOptions, until: Instant) -> ComposedStreamRun {
    f.admit(route, until).await.unwrap().composed_stream(payload, options)
}

async fn composed_bytes(mut body: port::RawStreamBody) -> Vec<u8> {
    let mut bytes = Vec::new();
    tokio::time::timeout(STEP, async {
        while let Some(chunk) = body.next().await {
            let chunk = chunk.unwrap();
            assert!(bytes.len() + chunk.len() <= 1024 * 1024);
            bytes.extend_from_slice(&chunk);
        }
    }).await.unwrap();
    bytes
}

fn events(bytes: &[u8]) -> Vec<Value> {
    std::str::from_utf8(bytes).unwrap().split("\n\n")
        .filter_map(|frame| frame.lines().find_map(|line| line.strip_prefix("data: ")))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str(data).unwrap()).collect()
}

async fn composed_finished(run: ComposedStreamRun) -> (Vec<u8>, ComposedStreamCompletion) {
    let head = tokio::time::timeout(STEP, run.response.result).await.unwrap().unwrap();
    let bytes = composed_bytes(head.body).await;
    let result = tokio::time::timeout(STEP, run.finished).await.unwrap();
    cleanup(run.response.cleanup).await;
    (bytes, result)
}

fn completed_output(events: &[Value]) -> &Value {
    &events.iter().find(|v| v["type"] == "response.completed").unwrap()["response"]["output"]
}

async fn composed_outcome(mut body: port::RawStreamBody) -> (Vec<u8>, Option<FailureKind>) {
    tokio::time::timeout(STEP, async move {
        let mut bytes = Vec::new();
        while let Some(chunk) = body.next().await {
            match chunk {
                Ok(chunk) => {
                    assert!(bytes.len() + chunk.len() <= 1024 * 1024);
                    bytes.extend_from_slice(&chunk);
                }
                Err(kind) => { assert_eq!(body.next().await, None); return (bytes, Some(kind)); }
            }
        }
        (bytes, None)
    }).await.unwrap()
}

async fn summary(value: port::ComposedStreamFinished) -> ComposedStreamCompletion {
    tokio::time::timeout(STEP, value).await.unwrap()
}

fn minimal_body() -> Value {
    json!({"id":"resp_exact-wire-id", "created_at":1234, "status":"completed",
        "output":[{"type":"message", "role":"assistant",
            "content":[{"type":"output_text", "text":"hello λ🙂"}]}],
        "usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}})
}

fn composed_hooked(f: &Fixture, until: Instant, options: ComposedStreamOptions,
    hooks: TestHooks) -> ComposedStreamRun {
    let mut call = call_for_hook(f, until);
    call.request.route = Route::Responses;
    super::composed_stream::launch_for_test(call, responses_payload(), options,
        Box::new(move |parts, payload| Ok(pending_with_hooks(parts, payload, hooks))))
}

fn no_success(events: &[Value]) {
    assert!(!events.iter().any(|event| matches!(event["type"].as_str(),
        Some("response.completed") | Some("response.incomplete") | Some("message_stop"))));
}
