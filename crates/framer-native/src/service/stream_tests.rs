use super::*;
use framer_core::convert::AssembledResponses;
use futures::{FutureExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::oneshot;

fn success() -> Outcome {
    Outcome { response: Some(CompletedJson::Responses(AssembledResponses {
        id: "upstream-id".into(), created_at: 123, status: "completed".into(),
        incomplete_details: None, service_tier: Value::Null,
        output: vec![json!({"type":"message", "content":[{"type":"output_text","text":"hello"}]}),
            json!({"type":"function_call", "id":"tool-1", "call_id":"call-1",
                "name":"run", "namespace":"fixture", "arguments":"{\"n\":1}", "status":"completed"})],
        usage: json!({"input_tokens":2,"output_tokens":3}),
        input_tokens: 2, output_tokens: 3, chunks: 1, metered_chunks: 0,
    })), evidence: None, failure: None, cleanup_confirmed: true }
}

fn output() -> ChainEvent {
    let Some(CompletedJson::Responses(value)) = success().response else { unreachable!() };
    ChainEvent::ResponsesItems { items: value.output, continues_text: false }
}

fn text(frame: bytes::Bytes) -> String { String::from_utf8(frame.to_vec()).unwrap() }
fn data(frames: &str) -> Vec<Value> {
    frames.lines().filter_map(|line| line.strip_prefix("data: "))
        .map(|line| serde_json::from_str(line).unwrap()).collect()
}

#[tokio::test]
async fn output_precedes_joined_completion_and_keeps_one_response_identity() {
    let (done, joined) = oneshot::channel();
    let (send, receive) = mpsc::channel(1);
    let response = response(async { joined.await.unwrap() }.boxed(), receive,
        FreeRoute::Responses, "fixture-model".into(), uuid::Uuid::new_v4());
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body().into_data_stream();
    let opening = text(body.next().await.unwrap().unwrap());
    let id = data(&opening)[0]["response"]["id"].clone();
    assert!(!opening.contains("hello"));
    send.send(output()).await.unwrap();
    let chunk = text(body.next().await.unwrap().unwrap());
    assert!(chunk.contains("response.output_text.delta") && chunk.contains("hello"));
    assert!(chunk.contains("response.function_call_arguments.done"));
    assert!(chunk.contains("\"namespace\":\"fixture\""));
    assert!(!chunk.contains("response.completed"));
    drop(send);
    assert!(body.next().now_or_never().is_none());
    done.send(success()).ok().unwrap();
    let final_chunk = text(body.next().await.unwrap().unwrap());
    let completed = data(&final_chunk).pop().unwrap();
    assert_eq!(completed["type"], "response.completed");
    assert_eq!(completed["response"]["id"], id);
    assert_eq!(completed["response"]["usage"]["output_tokens"], 3);
    assert!(body.next().await.is_none());
}

#[tokio::test]
async fn cleanup_failure_after_output_never_emits_success() {
    for route in [FreeRoute::Responses, FreeRoute::Messages] {
        let (done, joined) = oneshot::channel();
        let (send, receive) = mpsc::channel(1);
        let mut body = response(async { joined.await.unwrap() }.boxed(), receive,
            route, "fixture-model".into(), uuid::Uuid::new_v4()).into_body().into_data_stream();
        body.next().await.unwrap().unwrap();
        let event = match route {
            FreeRoute::Responses => output(),
            FreeRoute::Messages => ChainEvent::Chunk(vec![json!({"type":"text","text":"hello"})]),
        };
        send.send(event).await.unwrap();
        assert!(text(body.next().await.unwrap().unwrap()).contains("hello"));
        let mut failed = success();
        failed.cleanup_confirmed = false;
        done.send(failed).ok().unwrap();
        let tail = text(body.next().await.unwrap().unwrap());
        assert!(tail.contains("Cleanup"));
        assert!(!tail.contains("response.completed") && !tail.contains("message_stop"));
        assert!(!tail.contains("upstream-id"));
        assert!(body.next().await.is_none());
    }
}

#[tokio::test]
async fn simultaneous_message_and_completion_drain_output_before_terminal() {
    let (send, receive) = mpsc::channel(1);
    let blocks = vec![json!({"type":"text","text":"hello"})];
    send.send(ChainEvent::Chunk(blocks.clone())).await.unwrap();
    let outcome = Outcome {
        response: Some(CompletedJson::Messages(convert::AssembledResponse {
            id: "fixture-message".into(), content: blocks, stop_reason: "end_turn".into(),
            stop_sequence: None, usage: json!({}), input_tokens: 1, output_tokens: 2,
            chunks: 1, metered_chunks: 0,
        })), evidence: None, failure: None, cleanup_confirmed: true,
    };
    let mut body = response(futures::future::ready(outcome).boxed(), receive,
        FreeRoute::Messages, "fixture-model".into(), uuid::Uuid::new_v4()).into_body().into_data_stream();
    assert!(text(body.next().await.unwrap().unwrap()).contains("message_start"));
    let tail = text(body.next().await.unwrap().unwrap());
    assert!(tail.find("hello").unwrap() < tail.find("message_stop").unwrap());
    assert_eq!(tail.matches("event: message_stop").count(), 1);
    assert!(body.next().await.is_none());
}

#[tokio::test]
async fn known_failure_discards_undelivered_output() {
    let (send, receive) = mpsc::channel(1);
    send.send(output()).await.unwrap();
    let mut outcome = success();
    outcome.failure = Some(FailureKind::Deadline);
    let mut body = response(futures::future::ready(outcome).boxed(), receive,
        FreeRoute::Responses, "fixture-model".into(), uuid::Uuid::new_v4()).into_body().into_data_stream();
    body.next().await.unwrap().unwrap();
    let tail = text(body.next().await.unwrap().unwrap());
    assert!(tail.contains("Deadline"));
    assert!(!tail.contains("hello") && !tail.contains("response.completed"));
    assert!(body.next().await.is_none());
}
