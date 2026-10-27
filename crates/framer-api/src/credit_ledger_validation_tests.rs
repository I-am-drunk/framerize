use super::usage;
use crate::{session_auth::Egress, transport::Transport};
use framer_core::{port::Session, workspace::CreditSnapshot, FramerUserId, TeamId};
use serde_json::{json, Value};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const LIMIT: Duration = Duration::from_secs(3);
const SERVER_LIMIT: Duration = Duration::from_secs(6);
const AUTH_PATH: &str = "/auth/web/access-token";
const LEDGER_PATH: &str = "/web/teams/fixture-ledger-team/ai-credits";
const INVALID: &str = "bad credit ledger response: invalid availableCredits";
const COOKIE: &str = "ledger-validation=fixture-cookie";
const TOKEN: &str = "fixture-ledger-web-token";

struct Reply { status: u16, body: String }

impl Reply {
    fn new(status: u16, body: impl Into<String>) -> Self {
        Self { status, body: body.into() }
    }
}

fn auth_reply() -> Reply {
    Reply::new(200, json!({
        "accessToken": TOKEN, "expiresAt": "2099-01-01T00:00:00Z"
    }).to_string())
}

#[derive(Default)]
struct Observation {
    connections: usize,
    auth_calls: usize,
    ledger_calls: usize,
    order: Vec<&'static str>,
    wire_ok: bool,
}

struct Fixture {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<Observation>>>,
}

impl Fixture {
    fn start(auth: Reply, ledger: Reply) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let worker = thread::spawn(move || serve(listener, signal, auth, ledger));
        Self { address, stop, worker: Some(worker) }
    }

    fn finish(mut self) -> Observation {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().expect("fixture worker panicked")
            .expect("fixture socket failed")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
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
            Err(e) if matches!(e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => continue,
            Err(e) => return Err(e),
        }
        if bytes.len() > 8192 { return Err(invalid("fixture input limit")); }
        if bytes.windows(4).any(|part| part == b"\r\n\r\n") { return Ok(Some(bytes)); }
    }
    Ok(None)
}

fn wire_matches(bytes: &[u8], path: &str, cookie: bool) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let lower = text.to_ascii_lowercase();
    let auth_ok = if cookie {
        lower.contains(&format!("\r\ncookie: {COOKIE}\r\n"))
            && !lower.contains("\r\nauthorization:")
    } else {
        lower.contains(&format!("\r\nauthorization: bearer {TOKEN}\r\n"))
            && !lower.contains("\r\ncookie:")
    };
    text.starts_with(&format!("GET {path} HTTP/1.1\r\n"))
        && lower.contains("\r\nx-requested-by: framer\r\n")
        && lower.contains("\r\nuser-agent: framerize/0.1\r\n")
        && auth_ok
}

fn serve(listener: TcpListener, stop: Arc<AtomicBool>, auth: Reply,
    ledger: Reply) -> io::Result<Observation> {
    let end = Instant::now() + SERVER_LIMIT;
    let mut observed = Observation { wire_ok: true, ..Observation::default() };
    while !stop.load(Ordering::SeqCst) && Instant::now() < end {
        let (mut stream, _) = match listener.accept() {
            Ok(connection) => connection,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
                continue;
            }
            Err(e) => return Err(e),
        };
        observed.connections += 1;
        if observed.connections > 8 { return Err(invalid("fixture connection limit")); }
        let Some(bytes) = read_request(&mut stream, &stop, end)? else { continue };
        let request = String::from_utf8_lossy(&bytes);
        let path = request.split_whitespace().nth(1).unwrap_or("");
        let reply = match path {
            AUTH_PATH => {
                observed.auth_calls += 1;
                observed.order.push("auth");
                observed.wire_ok &= wire_matches(&bytes, path, true);
                &auth
            }
            LEDGER_PATH => {
                observed.ledger_calls += 1;
                observed.order.push("ledger");
                observed.wire_ok &= wire_matches(&bytes, path, false);
                &ledger
            }
            _ => return Err(invalid("unexpected fixture path")),
        };
        if reply.body.len() > 8192 { return Err(invalid("fixture response limit")); }
        let response = format!(
            "HTTP/1.1 {} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            reply.status, reply.body.len(), reply.body);
        stream.write_all(response.as_bytes())?;
    }
    Ok(observed)
}

fn exchange(auth: Reply, ledger: Reply) -> (Result<CreditSnapshot, String>, Observation) {
    let fixture = Fixture::start(auth, ledger);
    let transport = Transport::control_for_loopback(fixture.address, Egress::Direct).unwrap();
    let session = Session {
        active_user: FramerUserId("fixture-ledger-user".into()),
        cookie_header: COOKIE.into(),
    };
    let team = TeamId("fixture-ledger-team".into());
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let result = runtime.block_on(async {
        tokio::time::timeout(LIMIT, usage(&transport, &session, &team)).await
    });
    drop(transport);
    drop(runtime);
    let observed = fixture.finish();
    (result.expect("ledger call exceeded fixture bound"), observed)
}

fn ledger(body: Value) -> (Result<CreditSnapshot, String>, Observation) {
    exchange(auth_reply(), Reply::new(200, body.to_string()))
}

fn assert_wire(observed: Observation, ledger_calls: usize) {
    assert_eq!(observed.connections, 1 + ledger_calls);
    assert_eq!(observed.auth_calls, 1);
    assert_eq!(observed.ledger_calls, ledger_calls);
    assert_eq!(observed.order, if ledger_calls == 0 { vec!["auth"] }
        else { vec!["auth", "ledger"] });
    assert!(observed.wire_ok, "request path, authentication or headers changed");
}

fn assert_snapshot(snapshot: CreditSnapshot, available: f64,
    funds: u32, grants: u32, allocated: f64) {
    assert_eq!(snapshot.available.to_bits(), available.to_bits());
    assert_eq!(snapshot.funds, funds);
    assert_eq!(snapshot.grants, grants);
    assert_eq!(snapshot.allocated, allocated);
    assert_eq!(snapshot.looks_exhausted(), available <= 0.0);
}

#[test]
fn missing_available_is_rejected() {
    for body in [json!({}), json!({
        "funds": [{"grants": [{"allocatedCredits": 500}]}], "unknown": true
    }), json!([]), Value::Null] {
        let (result, observed) = ledger(body);
        assert_eq!(result.unwrap_err(), INVALID);
        assert_wire(observed, 1);
    }
}

#[test]
fn nonnumeric_available_is_rejected() {
    for available in [Value::Null, json!("0"), json!(false), json!([]),
        json!({}), json!("NaN"), json!("Infinity")] {
        let (result, observed) = ledger(json!({
            "availableCredits": available,
            "funds": [{"grants": [{"allocatedCredits": 500}]}]
        }));
        assert_eq!(result.unwrap_err(), INVALID);
        assert_wire(observed, 1);
    }
}

#[test]
fn zero_and_negative_available_keep_exhaustion() {
    for available in [0.0_f64, -0.0, -12.5] {
        let (result, observed) = ledger(json!({"availableCredits": available}));
        assert_snapshot(result.unwrap(), available, 0, 0, 0.0);
        assert_wire(observed, 1);
    }
}

#[test]
fn positive_available_preserves_snapshot() {
    let detailed = json!({
        "availableCredits": 500.25,
        "funds": [
            {"grants": [{"allocatedCredits": 500}, {"allocatedCredits": 125.5}]},
            {"grants": [{"allocatedCredits": -2}, {"allocatedCredits": null}, {}]},
            {"grants": "unknown", "extra": true}
        ],
        "unknown": {"nested": true}
    });
    for (body, available, funds, grants, allocated) in [
        (detailed, 500.25, 3, 5, 623.5),
        (json!({"availableCredits": 3.125}), 3.125, 0, 0, 0.0),
        (json!({"availableCredits": 0.5, "funds": "unknown"}), 0.5, 0, 0, 0.0),
        (json!({"availableCredits": f64::MAX}), f64::MAX, 0, 0, 0.0),
    ] {
        let (result, observed) = ledger(body);
        assert_snapshot(result.unwrap(), available, funds, grants, allocated);
        assert_wire(observed, 1);
    }
}

#[test]
fn http_and_json_errors_keep_precedence() {
    // None denotes the unchanged reqwest JSON-decoding error, before validation.
    let cases = [
        (Reply::new(401, "{}"), Reply::new(200, "{}"),
            Some("unauthorized".to_owned()), 0),
        (Reply::new(403, "limited"), Reply::new(200, "{}"),
            Some(format!("retryable 403 on {AUTH_PATH}: limited")), 0),
        (Reply::new(500, "unavailable"), Reply::new(200, "{}"),
            Some(format!("retryable 500 on {AUTH_PATH}: unavailable")), 0),
        (Reply::new(200, "not-json"), Reply::new(200, "{}"), None, 0),
        (Reply::new(200, "{}"), Reply::new(200, "{}"),
            Some("bad access-token response".to_owned()), 0),
        (Reply::new(200, json!({"accessToken": TOKEN, "expiresAt": "invalid"}).to_string()),
            Reply::new(200, "{}"), Some("bad expiresAt".to_owned()), 0),
        (auth_reply(), Reply::new(401, "{}"), Some("unauthorized".to_owned()), 1),
        (auth_reply(), Reply::new(402, "{}"), Some("credits-exhausted".to_owned()), 1),
        (auth_reply(), Reply::new(403, "limited"),
            Some(format!("retryable 403 on {LEDGER_PATH}: limited")), 1),
        (auth_reply(), Reply::new(500, "unavailable"),
            Some(format!("retryable 500 on {LEDGER_PATH}: unavailable")), 1),
        (auth_reply(), Reply::new(200, "{\"availableCredits\":"), None, 1),
        (auth_reply(), Reply::new(200, "{\"availableCredits\":1e400}"), None, 1),
    ];
    for (auth, ledger, expected, ledger_calls) in cases {
        let (result, observed) = exchange(auth, ledger);
        let error = result.unwrap_err();
        match expected {
            Some(expected) => assert_eq!(error, expected),
            None => {
                assert!(error.starts_with("error decoding response body"),
                    "JSON failure changed: {error}");
                assert_ne!(error, INVALID);
            }
        }
        assert_wire(observed, ledger_calls);
    }
}
