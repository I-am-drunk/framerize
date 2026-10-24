use super::*;
use futures::{FutureExt, StreamExt};
use serde_json::json;

#[path = "../native_dispatch/loopback.rs"]
mod loopback;
use loopback::{Expected, Reply, Server, STEP, TICK};

const PATH: &str = "/ai/v3/openai/responses/";

fn run(work: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(work);
}

fn inputs(address: std::net::SocketAddr) -> (NativeHttp, AccountId, Session, Value) {
    inputs_for_user(address, "fixture-user")
}

fn inputs_for_user(address: std::net::SocketAddr, user: &str)
    -> (NativeHttp, AccountId, Session, Value) {
    let account = AccountId(uuid::Uuid::new_v4());
    let user = FramerUserId(user.into());
    let client = NativeHttp::for_loopback(account, user.clone(), address,
        NativeHttpLimits { request_timeout: STEP, connect_timeout: STEP, ..Default::default() }).unwrap();
    (client, account, Session { active_user: user,
        cookie_header: "session=fixture-application".into() }, json!({"stream": true}))
}

fn token(account: AccountId) -> FreshCastleToken {
    token_with_value(account, "fixture-token")
}

fn token_with_value(account: AccountId, value: &str) -> FreshCastleToken {
    FreshCastleToken::from_sdk_output(account, CastleToken(value.into()))
}

async fn wait_for(count: &AtomicUsize) {
    tokio::time::timeout(STEP, async {
        while count.load(Ordering::Acquire) == 0 { tokio::time::sleep(TICK).await; }
    }).await.unwrap();
}

#[test]
fn inference_retains_bearer_and_native_origin_without_session_cookies() {
    let server = Server::new(vec![Expected { path: PATH,
        envelope: json!({"stream": true, "castleToken": "fixture-token"}),
        reply: Reply::json(json!({"fixture": "complete"})) }]);
    let mut received = None;
    run(async {
        let (client, account, session, body) = inputs(server.address);
        let response = client.ai_post(&account, PATH,
            AccountRequestAuth::new(account, &session, "fixture-application-bearer"),
            &body, token(account), NativeCancellation::new()).await;
        assert_eq!(client.in_flight.load(Ordering::Acquire), 0);
        received = Some(response);
    });
    let ended = server.finish();
    assert_eq!(ended.fault, None);
    assert_eq!((ended.accepted, ended.requests, ended.headers, ended.replies), (1, 1, 1, 1));
    let response = received.unwrap().unwrap();
    assert_eq!(response.0, 200);
    assert_eq!(serde_json::from_str::<Value>(&response.2).unwrap(),
        json!({"fixture": "complete"}));
}

#[test]
fn resident_control_sends_exact_current_jar_and_serving_headers_once() {
    let server = Server::with_resident_headers(vec![Expected { path: PATH,
        envelope: json!({"stream": true, "castleToken": "fixture-token"}),
        reply: Reply::json(json!({"fixture": "complete"})) }], false);
    run(async {
        let (client, account, mut session, body) = inputs(server.address);
        session.cookie_header = "session=fixture-application; __cuid=fixture-device; fixture=one; fixture=two".into();
        let client = client.with_resident_browser_headers();
        let response = client.ai_post(&account, PATH,
            AccountRequestAuth::new(account, &session, "fixture-application-bearer"),
            &body, token(account), NativeCancellation::new()).await.unwrap();
        assert_eq!(response.0, 200);
        assert_eq!(client.in_flight.load(Ordering::Acquire), 0);
    });
    let ended = server.finish();
    assert_eq!(ended.fault, None);
    assert_eq!((ended.accepted, ended.requests, ended.headers, ended.replies), (1, 1, 1, 1));
}

#[test]
fn send_is_counted_before_headers_and_cancellation_joins() {
    let server = Server::with_held_headers(vec![Expected { path: PATH,
        envelope: json!({"stream": true, "castleToken": "fixture-token"}),
        reply: Reply::json(json!({})) }]);
    run(async {
        let (client, account, session, body) = inputs(server.address);
        let cancellation = NativeCancellation::new();
        let before = send_evidence::total();
        let NativeTrackedRequest { mut response, closed } = client.ai_post_stream_tracked(
            &account, PATH, AccountRequestAuth::new(account, &session, "fixture-application-bearer"),
            &body, token(account), cancellation.clone());
        tokio::time::timeout(STEP, async {
            loop {
                assert!((&mut response).now_or_never().is_none());
                if server.shared.requests.load(Ordering::Acquire) > 0 { break; }
                tokio::time::sleep(TICK).await;
            }
        }).await.unwrap();
        assert!(send_evidence::total() > before);
        assert_eq!(server.shared.headers.load(Ordering::Acquire), 0);
        cancellation.cancel();
        assert_eq!(response.await.err().as_deref(), Some(CANCELLED));
        assert_eq!(closed.await, Ok(()));
        wait_for(&server.shared.peer_eof).await;
        assert_eq!(client.in_flight.load(Ordering::Acquire), 0);
    });
    let ended = server.finish();
    assert_eq!((ended.accepted, ended.requests, ended.headers, ended.peer_eof), (1, 1, 0, 1));
    assert_eq!(ended.fault, None);
}

#[test]
fn resident_control_cancellation_still_joins_without_response_headers() {
    let server = Server::with_resident_headers(vec![Expected { path: PATH,
        envelope: json!({"stream": true, "castleToken": "fixture-token"}),
        reply: Reply::json(json!({})) }], true);
    run(async {
        let (client, account, mut session, body) = inputs(server.address);
        session.cookie_header = "session=fixture-application; __cuid=fixture-device; fixture=one; fixture=two".into();
        let client = client.with_resident_browser_headers();
        let cancel = NativeCancellation::new();
        let NativeTrackedRequest { mut response, closed } = client.ai_post_stream_tracked(
            &account, PATH, AccountRequestAuth::new(account, &session, "fixture-application-bearer"),
            &body, token(account), cancel.clone());
        tokio::time::timeout(STEP, async {
            while server.shared.requests.load(Ordering::Acquire) == 0 {
                assert!((&mut response).now_or_never().is_none());
                tokio::time::sleep(TICK).await;
            }
        }).await.unwrap();
        cancel.cancel();
        assert_eq!(response.await.err().as_deref(), Some(CANCELLED));
        assert_eq!(closed.await, Ok(()));
        wait_for(&server.shared.peer_eof).await;
        assert_eq!(client.in_flight.load(Ordering::Acquire), 0);
    });
    let ended = server.finish();
    assert_eq!(ended.fault, None);
    assert_eq!((ended.accepted, ended.requests, ended.headers, ended.peer_eof), (1, 1, 0, 1));
}

#[test]
fn cancelling_one_account_preserves_another_accounts_request() {
    let expected = |castle: &str| vec![Expected { path: PATH,
        envelope: json!({"stream": true, "castleToken": castle}),
        reply: Reply::json(json!({"fixture": "complete"})) }];
    let first_server = Server::with_held_headers_and_authorization(
        expected("fixture-first-token"), "Bearer fixture-first-bearer");
    let second_server = Server::with_held_headers_and_authorization(
        expected("fixture-second-token"), "Bearer fixture-second-bearer");
    run(async {
        let (first, first_account, first_session, first_body) =
            inputs_for_user(first_server.address, "fixture-first-user");
        let (second, second_account, second_session, second_body) =
            inputs_for_user(second_server.address, "fixture-second-user");
        assert_ne!(first_account, second_account);
        assert_ne!(first_session.active_user, second_session.active_user);
        let first_cancel = NativeCancellation::new();
        let mut first_request = first.ai_post_stream_tracked(&first_account, PATH,
            AccountRequestAuth::new(first_account, &first_session, "fixture-first-bearer"),
            &first_body, token_with_value(first_account, "fixture-first-token"), first_cancel.clone());
        let mut second_request = second.ai_post_stream_tracked(&second_account, PATH,
            AccountRequestAuth::new(second_account, &second_session, "fixture-second-bearer"),
            &second_body, token_with_value(second_account, "fixture-second-token"), NativeCancellation::new());
        tokio::time::timeout(STEP, async {
            loop {
                assert!((&mut first_request.response).now_or_never().is_none());
                assert!((&mut second_request.response).now_or_never().is_none());
                if first_server.shared.requests.load(Ordering::Acquire) == 1
                    && second_server.shared.requests.load(Ordering::Acquire) == 1 { break; }
                tokio::time::sleep(TICK).await;
            }
        }).await.unwrap();
        assert_eq!(first.in_flight.load(Ordering::Acquire), 1);
        assert_eq!(second.in_flight.load(Ordering::Acquire), 1);
        for (auth_account, session, token_account, error) in [
            (first_account, &first_session, second_account, "native-http: account mismatch"),
            (second_account, &first_session, second_account, "native-http: session identity mismatch"),
            (second_account, &second_session, first_account, "native-http: account mismatch"),
        ] {
            let crossed = second.ai_post_stream_tracked(&second_account, PATH,
                AccountRequestAuth::new(auth_account, session, "fixture-second-bearer"),
                &second_body, token_with_value(token_account, "fixture-second-token"), NativeCancellation::new());
            assert_eq!(crossed.response.await.err().as_deref(), Some(error));
            assert_eq!(crossed.closed.await, Ok(()));
        }
        first_cancel.cancel();
        assert_eq!(first_request.response.await.err().as_deref(), Some(CANCELLED));
        assert_eq!(first_request.closed.await, Ok(()));
        wait_for(&first_server.shared.peer_eof).await;
        assert_eq!(first.in_flight.load(Ordering::Acquire), 0);
        assert_eq!(second.in_flight.load(Ordering::Acquire), 1);
        assert!((&mut second_request.response).now_or_never().is_none());
        assert!((&mut second_request.closed).now_or_never().is_none());
        assert_eq!(second_server.shared.requests.load(Ordering::Acquire), 1);
        assert_eq!(second_server.shared.headers.load(Ordering::Acquire), 0);
        second_server.shared.release_headers.store(true, Ordering::Release);
        let mut response = tokio::time::timeout(STEP, second_request.response)
            .await.unwrap().unwrap();
        assert_eq!(response.status, 200);
        let mut received = Vec::new();
        tokio::time::timeout(STEP, async {
            while let Some(chunk) = response.body.next().await {
                received.extend_from_slice(&chunk.unwrap());
            }
        }).await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&received).unwrap(),
            json!({"fixture": "complete"}));
        assert_eq!(second_request.closed.await, Ok(()));
        assert_eq!(second.in_flight.load(Ordering::Acquire), 0);
    });
    let first_ended = first_server.finish();
    let second_ended = second_server.finish();
    assert_eq!(first_ended.fault, None);
    assert_eq!(second_ended.fault, None);
    assert_eq!((first_ended.accepted, first_ended.requests, first_ended.headers,
        first_ended.peer_eof), (1, 1, 0, 1));
    assert_eq!((second_ended.accepted, second_ended.requests, second_ended.headers,
        second_ended.replies), (1, 1, 1, 1));
}

#[test]
fn omitted_wire_cookie_still_requires_valid_local_credentials() {
    let server = Server::new(vec![]);
    run(async {
      for resident in [false, true] {
        let (client, account, mut session, body) = inputs(server.address);
        let client = if resident { client.with_resident_browser_headers() } else { client };
        for (header, expected) in [
            ("", "native-http: request credentials missing"),
            ("session=fixture\r\nx-injected: value", "native-http: invalid cookie header"),
        ] {
            session.cookie_header = header.into();
            let request = client.ai_post_stream_tracked(&account, PATH,
                AccountRequestAuth::new(account, &session, "fixture-application-bearer"),
                &body, token(account), NativeCancellation::new());
            assert_eq!(request.response.await.err().as_deref(), Some(expected));
            assert_eq!(request.closed.await, Ok(()));
            assert_eq!(client.in_flight.load(Ordering::Acquire), 0);
        }
      }
    });
    let ended = server.finish();
    assert_eq!((ended.accepted, ended.requests), (0, 0));
    assert_eq!(ended.fault, None);
}

#[test]
fn transport_failure_without_headers_is_counted_and_closed() {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    run(async {
        let (client, account, session, body) = inputs(address);
        let before = send_evidence::total();
        let request = client.ai_post_stream_tracked(&account, PATH,
            AccountRequestAuth::new(account, &session, "fixture-application-bearer"),
            &body, token(account), NativeCancellation::new());
        assert_eq!(request.response.await.err().as_deref(), Some("native-http: transport failed"));
        assert!(send_evidence::total() > before);
        assert_eq!(request.closed.await, Ok(()));
        assert_eq!(client.in_flight.load(Ordering::Acquire), 0);
    });
}

#[test]
fn unpolled_precancelled_and_invalid_requests_never_connect() {
    let server = Server::new(vec![]);
    run(async {
        let (client, account, session, body) = inputs(server.address);
        let request = client.ai_post_stream_tracked(&account, PATH,
            AccountRequestAuth::new(account, &session, "fixture-application-bearer"),
            &body, token(account), NativeCancellation::new());
        drop(request.response);
        assert_eq!(request.closed.await, Ok(()));
        let cancellation = NativeCancellation::new();
        cancellation.cancel();
        let request = client.ai_post_stream_tracked(&account, PATH,
            AccountRequestAuth::new(account, &session, "fixture-application-bearer"),
            &body, token(account), cancellation);
        assert_eq!(request.response.await.err().as_deref(), Some(CANCELLED));
        assert_eq!(request.closed.await, Ok(()));
        let request = client.ai_post_stream_tracked(&account, "/not-an-inference-route",
            AccountRequestAuth::new(account, &session, "fixture-application-bearer"),
            &body, token(account), NativeCancellation::new());
        assert_eq!(request.response.await.err().as_deref(), Some("native-http: unsupported AI route"));
        assert_eq!(request.closed.await, Ok(()));
        assert_eq!(client.in_flight.load(Ordering::Acquire), 0);
    });
    let ended = server.finish();
    assert_eq!((ended.accepted, ended.requests), (0, 0));
    assert_eq!(ended.fault, None);
}
