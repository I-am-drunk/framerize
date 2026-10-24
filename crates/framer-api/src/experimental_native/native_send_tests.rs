use super::*;
use futures::FutureExt;
use serde_json::json;

#[path = "../native_dispatch/loopback.rs"]
mod loopback;
use loopback::{Expected, Reply, Server, STEP, TICK};

const PATH: &str = "/ai/v3/openai/responses/";

fn run(work: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(work);
}

fn inputs(address: std::net::SocketAddr) -> (NativeHttp, AccountId, Session, Value) {
    let account = AccountId(uuid::Uuid::new_v4());
    let user = FramerUserId("fixture-user".into());
    let client = NativeHttp::for_loopback(account, user.clone(), address,
        NativeHttpLimits { request_timeout: STEP, connect_timeout: STEP, ..Default::default() }).unwrap();
    (client, account, Session { active_user: user,
        cookie_header: "session=fixture-application".into() }, json!({"stream": true}))
}

fn token(account: AccountId) -> FreshCastleToken {
    FreshCastleToken::from_sdk_output(account, CastleToken("fixture-token".into()))
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
fn omitted_wire_cookie_still_requires_valid_local_credentials() {
    let server = Server::new(vec![]);
    run(async {
        let (client, account, mut session, body) = inputs(server.address);
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
