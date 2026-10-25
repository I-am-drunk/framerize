use super::*;
use crate::request::complete_json::{FreeRoute, Plan};

#[test]
fn stream_waits_for_upstream_body_and_uses_the_same_single_attempt() {
    let mut reply = Reply::json(response_body());
    reply.gate_body = true;
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let plan = Plan::new(f.scope.clone(), specification(&f,
            FreeRoute::Responses, responses_payload())).unwrap().with_events(Some(sender));
        let probe = plan.launch_in_pool(f.client.clone(), f.supplier.clone(),
            f.capacity.clone().try_acquire_owned().unwrap(), f.admission()).await.unwrap();
        wait_count(&server.shared.headers, 1).await;
        assert!(receiver.try_recv().is_err());
        server.shared.release_body.store(true, Ordering::Release);
        let event = tokio::time::timeout(STEP, receiver.recv()).await.unwrap().unwrap();
        let framer_core::convert::ChainEvent::ResponsesItems { items, continues_text } = event
            else { panic!("wrong response dialect") };
        assert_eq!(items, response_body()["output"].as_array().unwrap().clone());
        assert!(!continues_text);
        assert!(probe_result(probe).await.accepted);
        assert!(receiver.recv().await.is_none());
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn incomplete_tool_output_is_not_published_on_either_stream() {
    for route in [FreeRoute::Responses, FreeRoute::Messages] {
        let (path, payload, expected_payload, mut body) = match route {
            FreeRoute::Responses => ("/ai/v3/responses/", responses_payload(),
                expected_responses(), response_body()),
            FreeRoute::Messages => ("/ai/v3/messages/", messages_payload(),
                expected_messages(), message_body()),
        };
        body["status"] = "incomplete".into();
        body["incomplete_details"] = json!({"reason":"max_output_tokens"});
        body["stop_reason"] = "max_tokens".into();
        let server = Server::new(vec![expected(path, expected_payload, 0, Reply::json(body))]);
        run(async {
            let f = Fixture::new(&server, Supplier::default());
            let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
            let plan = Plan::new(f.scope.clone(), specification(&f, route, payload))
                .unwrap().with_events(Some(sender));
            let probe = plan.launch_in_pool(f.client.clone(), f.supplier.clone(),
                f.capacity.clone().try_acquire_owned().unwrap(), f.admission()).await.unwrap();
            assert!(!probe_result(probe).await.accepted);
            assert!(receiver.recv().await.is_none());
            f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

#[test]
fn stream_rejects_output_with_unaccepted_http_metadata() {
    for (status, content_type) in [(202, Some("application/json")),
        (200, None), (200, Some("text/plain"))] {
        let mut reply = Reply::json(response_body());
        reply.status = status;
        reply.content_type = content_type;
        let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, reply)]);
        run(async {
            let f = Fixture::new(&server, Supplier::default());
            let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
            let plan = Plan::new(f.scope.clone(), specification(&f,
                FreeRoute::Responses, responses_payload())).unwrap().with_events(Some(sender));
            let probe = plan.launch_in_pool(f.client.clone(), f.supplier.clone(),
                f.capacity.clone().try_acquire_owned().unwrap(), f.admission()).await.unwrap();
            assert!(!probe_result(probe).await.accepted);
            assert!(receiver.recv().await.is_none());
            f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}
