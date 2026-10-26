use super::*;
use serde_json::Value;

fn events(wire: &str) -> Vec<Value> {
    wire.lines().filter_map(|line| line.strip_prefix("data: "))
        .map(|line| serde_json::from_str(line).unwrap()).collect()
}

fn assert_sequence(events: &[Value]) {
    let actual: Vec<_> = events.iter().map(|event| event["sequence_number"].as_u64()).collect();
    let expected: Vec<_> = (0..events.len() as u64).map(Some).collect();
    assert!(!events.is_empty());
    assert_eq!(actual, expected);
}

async fn run_reply(reply: Result<Reply, String>) -> String {
    let mut f = Fixture::new();
    let response = chain(f.state.clone(), f.headers(0), false, true).await.unwrap();
    let mut body = response.into_body().into_data_stream();
    let first = tokio::time::timeout(WAIT, body.next()).await.unwrap().unwrap().unwrap();
    let mut wire = String::from_utf8(first.to_vec()).unwrap();
    let call = f.next().await;
    assert!(call.reply.send(reply).is_ok());
    tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
    while let Some(frame) = tokio::time::timeout(WAIT, body.next()).await.unwrap() {
        wire.push_str(&String::from_utf8(frame.unwrap().to_vec()).unwrap());
    }
    available(&f.state.gate, &f.key(0, RESPONSE_MODEL)).await;
    f.no_call();
    wire
}

#[tokio::test]
async fn responses_handler_sequences_prelude_tools_and_final_snapshot() {
    let Reply::Responses(mut result) = success("responses") else { unreachable!() };
    result.output.push(json!({"type":"function_call", "id":"tool-kept", "call_id":"call-one",
        "name":"calculate", "arguments":"{}", "status":"completed"}));
    let wire = run_reply(Ok(Reply::Responses(result))).await;
    let frames = events(&wire);
    assert_sequence(&frames);
    let tools: Vec<_> = frames.iter().filter(|event| matches!(event["type"].as_str(),
        Some("response.function_call_arguments.delta" | "response.function_call_arguments.done"))).collect();
    assert_eq!(tools.len(), 2);
    assert!(tools.iter().all(|event| event["item_id"] == "tool-kept"));
    assert_eq!(frames.last().unwrap()["type"], "response.completed");
    assert_eq!(frames.last().unwrap()["response"]["output"][1]["id"], "tool-kept");
}

#[tokio::test]
async fn responses_handler_error_uses_the_open_stream_sequence() {
    let wire = run_reply(Err("fixture response rejected".into())).await;
    let frames = events(&wire);
    assert_sequence(&frames);
    assert_eq!(frames.last().unwrap()["type"], "response.failed");
    assert!(!wire.contains("response.completed"));
}

#[tokio::test]
async fn responses_handler_join_failure_emits_a_fixed_sequenced_error() {
    let wire = run_reply(Ok(success("messages"))).await;
    let frames = events(&wire);
    assert_sequence(&frames);
    assert_eq!(frames.last().unwrap()["type"], "response.failed");
    assert_eq!(frames.last().unwrap()["response"]["error"]["message"], "upstream operation task failed");
    assert!(!wire.contains("wrong fixture reply"));
    assert!(!wire.contains("response.completed"));
}

#[tokio::test(start_paused = true)]
async fn responses_heartbeat_keeps_identity_sequence_and_joined_completion() {
    let mut f = Fixture::new();
    let response = chain(f.state.clone(), f.headers(0), false, true).await.unwrap();
    let mut body = response.into_body().into_data_stream();
    let first = body.next().await.unwrap().unwrap();
    let mut wire = String::from_utf8(first.to_vec()).unwrap();
    let id = events(&wire)[0]["response"]["id"].clone();
    let call = f.next().await;
    let heartbeat = body.next().await.unwrap().unwrap();
    let heartbeat = String::from_utf8(heartbeat.to_vec()).unwrap();
    wire.push_str(&heartbeat);
    assert!(call.reply.send(Ok(success("responses"))).is_ok());
    call.dropped.await.unwrap();
    while let Some(frame) = body.next().await {
        wire.push_str(&String::from_utf8(frame.unwrap().to_vec()).unwrap());
    }
    available(&f.state.gate, &f.key(0, RESPONSE_MODEL)).await;
    f.no_call();
    let pulse = events(&heartbeat);
    assert_eq!(pulse.len(), 1, "heartbeat must reach an SSE event consumer");
    assert_eq!(pulse[0]["type"], "response.in_progress");
    assert_eq!(pulse[0]["response"]["id"], id);
    let all = events(&wire);
    assert_sequence(&all);
    assert_eq!(all.last().unwrap()["type"], "response.completed");
}
