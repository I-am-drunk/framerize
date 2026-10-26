use super::*;

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
