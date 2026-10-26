use super::*;

#[tokio::test(start_paused = true)]
async fn heartbeat_preserves_partial_output_and_waits_for_joined_completion() {
    let (done, joined) = oneshot::channel();
    let (send, receive) = mpsc::channel(1);
    let mut body = response(async { joined.await.unwrap() }.boxed(), receive,
        FreeRoute::Responses, "fixture-model".into(), uuid::Uuid::new_v4())
        .into_body().into_data_stream();
    let mut wire = text(body.next().await.unwrap().unwrap());
    let id = data(&wire)[0]["response"]["id"].clone();
    send.send(output()).await.unwrap();
    wire.push_str(&text(body.next().await.unwrap().unwrap()));
    drop(send);
    let waiting = tokio::time::Instant::now();
    let heartbeat = text(body.next().await.unwrap().unwrap());
    assert!(waiting.elapsed() >= Duration::from_secs(8));
    wire.push_str(&heartbeat);
    done.send(success()).ok().unwrap();
    while let Some(frame) = body.next().await {
        wire.push_str(&text(frame.unwrap()));
    }
    let pulse = data(&heartbeat);
    assert_eq!(pulse.len(), 1, "heartbeat must reach an SSE event consumer");
    assert_eq!(pulse[0]["type"], "response.in_progress");
    assert_eq!(pulse[0]["response"]["id"], id);
    let all = data(&wire);
    assert_sequence(&all);
    let final_response = &all.last().unwrap()["response"];
    assert_eq!(final_response["output"][0]["content"][0]["text"], "hello");
    assert_eq!(final_response["output"][1]["id"], "tool-1");
    assert_eq!(all.last().unwrap()["type"], "response.completed");
}

#[tokio::test(start_paused = true)]
async fn messages_wait_keeps_ping_and_joined_failure() {
    let (done, joined) = oneshot::channel();
    let (send, receive) = mpsc::channel(1);
    let mut body = response(async { joined.await.unwrap() }.boxed(), receive,
        FreeRoute::Messages, "fixture-model".into(), uuid::Uuid::new_v4())
        .into_body().into_data_stream();
    assert!(text(body.next().await.unwrap().unwrap()).contains("event: message_start"));
    drop(send);
    let ping = text(body.next().await.unwrap().unwrap());
    assert!(ping.starts_with("event: ping\n"));
    assert_eq!(data(&ping), vec![json!({"type":"ping"})]);
    let mut outcome = success();
    outcome.cleanup_confirmed = false;
    done.send(outcome).ok().unwrap();
    let tail = text(body.next().await.unwrap().unwrap());
    assert!(tail.contains("Cleanup"));
    assert!(!tail.contains("message_stop"));
    assert!(body.next().await.is_none());
}

fn assert_sequence(events: &[Value]) {
    assert!(!events.is_empty());
    let actual: Vec<_> = events.iter().map(|event| event["sequence_number"].as_u64()).collect();
    let expected: Vec<_> = (0..events.len() as u64).map(Some).collect();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn open_items_and_joined_completion_share_one_sequence() {
    let (done, joined) = oneshot::channel();
    let (send, receive) = mpsc::channel(1);
    let mut body = response(async { joined.await.unwrap() }.boxed(), receive,
        FreeRoute::Responses, "fixture-model".into(), uuid::Uuid::new_v4())
        .into_body().into_data_stream();
    let mut wire = text(body.next().await.unwrap().unwrap());
    send.send(output()).await.unwrap();
    wire.push_str(&text(body.next().await.unwrap().unwrap()));
    drop(send);
    assert!(body.next().now_or_never().is_none());
    done.send(success()).ok().unwrap();
    while let Some(frame) = tokio::time::timeout(Duration::from_secs(2), body.next()).await.unwrap() {
        wire.push_str(&text(frame.unwrap()));
    }
    let events = data(&wire);
    assert_sequence(&events);
    let tool_events: Vec<_> = events.iter().filter(|event| matches!(event["type"].as_str(),
        Some("response.function_call_arguments.delta" | "response.function_call_arguments.done"))).collect();
    assert_eq!(tool_events.len(), 2);
    assert!(tool_events.iter().all(|event| event["item_id"] == "tool-1"));
    assert_eq!(events.last().unwrap()["type"], "response.completed");
}

#[tokio::test]
async fn joined_failure_keeps_sequence_without_finishing_pending_message() {
    for clean in [false, true] {
        let (done, joined) = oneshot::channel();
        let (send, receive) = mpsc::channel(1);
        let mut body = response(async { joined.await.unwrap() }.boxed(), receive,
            FreeRoute::Responses, "fixture-model".into(), uuid::Uuid::new_v4())
            .into_body().into_data_stream();
        let mut wire = text(body.next().await.unwrap().unwrap());
        send.send(ChainEvent::ResponsesItems {
            items: vec![json!({"type":"message", "content":[{"type":"output_text","text":"partial"}]})],
            continues_text: true,
        }).await.unwrap();
        wire.push_str(&text(body.next().await.unwrap().unwrap()));
        let mut outcome = success();
        outcome.cleanup_confirmed = clean;
        if clean { outcome.failure = Some(FailureKind::Deadline); }
        done.send(outcome).ok().unwrap();
        while let Some(frame) = tokio::time::timeout(Duration::from_secs(2), body.next()).await.unwrap() {
            wire.push_str(&text(frame.unwrap()));
        }
        let events = data(&wire);
        assert_sequence(&events);
        assert_eq!(events.last().unwrap()["type"], "response.failed");
        assert!(!events.iter().any(|event| matches!(event["type"].as_str(),
            Some("response.completed" | "response.output_item.done" | "response.output_text.done"))));
    }
}
