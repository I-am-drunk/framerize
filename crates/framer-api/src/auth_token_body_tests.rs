//! Inert token bodies and a bounded, joined web-mint fixture.
use super::{decode_local_token, decode_relay_token, web_access_token, AiToken};
use crate::{session_auth::Egress, transport::Transport};
use framer_core::{port::Session, FramerUserId};
use serde_json::{json, Value};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const USER: &str = "empty-bearer-user";
const COOKIE: &str = "empty-bearer-validation=fixture-cookie";
const FUTURE: &str = "2099-01-01T00:00:00Z";
const RECOVERED: &str = "fixture-recovered-token";
const WEB: &str = "/auth/web/access-token";
const WAIT: Duration = Duration::from_secs(3);

fn session() -> Session {
    Session { active_user: FramerUserId(USER.into()), cookie_header: COOKIE.into() }
}

fn relay_body(entry: Value) -> Value {
    let mut body = json!({"accounts": {"other-user": {
        "accessToken": "other-token", "expiresAt": FUTURE
    }}});
    body["accounts"][USER] = entry;
    body
}

fn assert_error(result: Result<AiToken, String>, expected: &str) {
    assert_eq!(result.err().as_deref(), Some(expected));
}

#[test]
fn local_empty_bearer_is_rejected() {
    for body in [json!({"accessToken": "", "expiresAt": FUTURE}),
        json!({"accessToken": ""}), json!({"accessToken": "", "expiresAt": null}),
        json!({"accessToken": "", "expiresAt": "not a date"})] {
        assert_error(decode_local_token(&body, &session()), "bad access-token response");
    }
}

#[test]
fn relay_empty_bearer_is_rejected() {
    for entry in [json!({"accessToken": "", "expiresAt": FUTURE}),
        json!({"accessToken": ""}), json!({"accessToken": "", "expiresAt": null}),
        json!({"accessToken": "", "expiresAt": "not a date"})] {
        assert_error(decode_relay_token(&relay_body(entry), &session()), "relay: no token for user");
    }
}

#[test]
fn missing_and_nonstring_bearers_preserve_errors() {
    for body in [json!({"expiresAt": "not a date"}), json!({"accessToken": null}),
        json!({"accessToken": false}), json!({"accessToken": 7}),
        json!({"accessToken": []}), json!({"accessToken": {}}), json!(null)] {
        assert_error(decode_local_token(&body, &session()), "bad access-token response");
        assert_error(decode_relay_token(&relay_body(body), &session()), "relay: no token for user");
    }
    let mut wrong_user = session();
    wrong_user.active_user = FramerUserId("missing-selected-user".into());
    let body = relay_body(json!({"accessToken": RECOVERED, "expiresAt": FUTURE}));
    assert_error(decode_relay_token(&body, &wrong_user), "relay: no token for user");
}

#[test]
fn nonempty_bearers_preserve_expiry_and_session() {
    let session = session();
    let expires = chrono::DateTime::parse_from_rfc3339(FUTURE).unwrap().timestamp_millis();
    for bearer in ["fixture-token", " ", " token \t", "\0", "unicode-雪"] {
        let entry = json!({"accessToken": bearer, "expiresAt": FUTURE, "unknown": [1, 2]});
        for result in [decode_local_token(&entry, &session),
            decode_relay_token(&relay_body(entry), &session)] {
            let token = result.unwrap();
            assert_eq!(token.bearer, bearer);
            assert_eq!(token.expires_at_ms, expires);
            assert!(token.usable_for(&session));
            let mut other_user = session.clone();
            other_user.active_user = FramerUserId("replacement-user".into());
            assert!(!token.usable_for(&other_user));
            let mut other_cookie = session.clone();
            other_cookie.cookie_header.push_str("-replacement");
            assert!(!token.usable_for(&other_cookie));
        }
    }
    for expiry in [None, Some(json!(null)), Some(json!(7)), Some(json!("not a date"))] {
        let mut entry = json!({"accessToken": RECOVERED});
        if let Some(expiry) = expiry { entry["expiresAt"] = expiry; }
        assert_error(decode_local_token(&entry, &session), "bad expiresAt");
        assert_error(decode_relay_token(&relay_body(entry), &session), "relay: bad expiresAt");
    }
    let near_expiry = chrono::DateTime::from_timestamp_millis(super::now_ms() + 10_000)
        .unwrap().to_rfc3339();
    for expiry in ["2000-01-01T00:00:00Z".to_owned(), near_expiry] {
        let entry = json!({"accessToken": RECOVERED, "expiresAt": expiry});
        let local = decode_local_token(&entry, &session).unwrap();
        assert_eq!(local.bearer, RECOVERED);
        assert!(!local.usable());
        assert_error(decode_relay_token(&relay_body(entry), &session),
            "relay: token is expired or within the refresh window");
    }
}

#[derive(Default)]
struct Observation {
    requests: usize,
    replies: usize,
    wire_ok: bool,
}

struct Fixture {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<Observation>>>,
}

impl Fixture {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let worker = thread::spawn(move || serve(listener, signal));
        Self { address, stop, worker: Some(worker) }
    }

    fn finish(mut self) -> io::Result<Observation> {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join()
            .map_err(|_| io::Error::other("fixture worker panicked"))?
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

fn read_request(stream: &mut TcpStream, stop: &AtomicBool, end: Instant)
    -> io::Result<Option<Vec<u8>>> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_millis(10)))?;
    stream.set_write_timeout(Some(Duration::from_millis(100)))?;
    let mut bytes = Vec::new();
    while !stop.load(Ordering::SeqCst) && Instant::now() < end {
        let mut buffer = [0; 1024];
        match stream.read(&mut buffer) {
            Ok(0) => return Ok(None),
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock
                | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => continue,
            Err(e) => return Err(e),
        }
        if bytes.len() > 8192 { return Err(io::Error::other("fixture input limit")); }
        if bytes.windows(4).any(|part| part == b"\r\n\r\n") { return Ok(Some(bytes)); }
    }
    Ok(None)
}

fn wire_matches(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let lower = text.to_ascii_lowercase();
    text.starts_with(&format!("GET {WEB} HTTP/1.1\r\n"))
        && lower.contains(&format!("\r\ncookie: {COOKIE}\r\n"))
        && !lower.contains("\r\nauthorization:")
        && lower.contains("\r\nx-requested-by: framer\r\n")
        && lower.contains("\r\nuser-agent: framerize/0.1\r\n")
}

fn serve(listener: TcpListener, stop: Arc<AtomicBool>) -> io::Result<Observation> {
    let end = Instant::now() + WAIT * 2;
    let mut observed = Observation { wire_ok: true, ..Observation::default() };
    let mut connections = 0;
    while !stop.load(Ordering::SeqCst) && Instant::now() < end {
        let (mut stream, _) = match listener.accept() {
            Ok(connection) => connection,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
                continue;
            }
            Err(e) => return Err(e),
        };
        connections += 1;
        if connections > 4 { return Err(io::Error::other("fixture connection limit")); }
        let Some(bytes) = read_request(&mut stream, &stop, end)? else { continue };
        observed.requests += 1;
        observed.wire_ok &= wire_matches(&bytes);
        let bearer = if observed.requests == 1 { "" } else { RECOVERED };
        let body = json!({"accessToken": bearer, "expiresAt": FUTURE}).to_string();
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
            Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        stream.write_all(response.as_bytes())?;
        observed.replies += 1;
    }
    if !stop.load(Ordering::SeqCst) { return Err(io::Error::other("fixture deadline")); }
    Ok(observed)
}

fn cached(transport: &Transport, session: &Session) -> Option<String> {
    transport.authorization.sessions.lock().unwrap()
        .get(&(session.cookie_header.clone(), WEB.to_owned())).map(|token| token.bearer.clone())
}

#[test]
fn empty_web_mint_is_not_cached() {
    let fixture = Fixture::start();
    let transport = Transport::control_for_loopback(fixture.address, Egress::Direct).unwrap();
    let session = session();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let outcome = runtime.block_on(async {
        tokio::time::timeout(WAIT, async {
            let first = web_access_token(&transport, &session).await;
            let first_cached = cached(&transport, &session);
            let second = web_access_token(&transport, &session).await;
            let second_cached = cached(&transport, &session);
            let third = web_access_token(&transport, &session).await;
            let third_cached = cached(&transport, &session);
            (first, first_cached, second, second_cached, third, third_cached)
        }).await
    });
    drop(transport);
    drop(runtime);
    let observed = fixture.finish();
    let (first, first_cached, second, second_cached, third, third_cached) =
        outcome.expect("web calls exceeded fixture bound");
    assert_error(first, "bad access-token response");
    assert!(first_cached.is_none());
    for result in [second, third] {
        let token = result.unwrap();
        assert_eq!(token.bearer, RECOVERED);
        assert!(token.usable_for(&session));
    }
    assert_eq!(second_cached.as_deref(), Some(RECOVERED));
    assert_eq!(third_cached.as_deref(), Some(RECOVERED));
    let observed = observed.expect("joined fixture failed");
    assert_eq!(observed.requests, 2);
    assert_eq!(observed.replies, 2);
    assert!(observed.wire_ok);
}
