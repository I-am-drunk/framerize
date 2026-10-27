//! Inert ledger sums through public usage, with bounded and joined HTTP work.
use super::usage;
use crate::{session_auth::Egress, transport::Transport};
use framer_core::{port::Session, workspace::CreditSnapshot, FramerUserId, TeamId};
use serde_json::{json, Value};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(3);
const AUTH: &str = "/auth/web/access-token";
const LEDGER: &str = "/web/teams/allocation-fixture-team/ai-credits";
const COOKIE: &str = "allocation-validation=fixture-cookie";
const TOKEN: &str = "allocation-fixture-token";
const LARGE: f64 = 1e308;
const INVALID: &str = "bad credit ledger response: invalid allocatedCredits total";

#[derive(Default)]
struct Observation {
    connections: usize,
    order: Vec<&'static str>,
    wire_ok: bool,
}

struct Fixture {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<Observation>>>,
}

impl Fixture {
    fn start(ledger: Value) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let worker = thread::spawn(move || serve(listener, signal, ledger.to_string()));
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

fn wire_matches(bytes: &[u8], path: &str, cookie: bool) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let lower = text.to_ascii_lowercase();
    let authorization = if cookie {
        lower.contains(&format!("\r\ncookie: {COOKIE}\r\n"))
            && !lower.contains("\r\nauthorization:")
    } else {
        lower.contains(&format!("\r\nauthorization: bearer {TOKEN}\r\n"))
            && !lower.contains("\r\ncookie:")
    };
    text.starts_with(&format!("GET {path} HTTP/1.1\r\n"))
        && lower.contains("\r\nx-requested-by: framer\r\n")
        && lower.contains("\r\nuser-agent: framerize/0.1\r\n")
        && authorization
}

fn serve(listener: TcpListener, stop: Arc<AtomicBool>, ledger: String)
    -> io::Result<Observation> {
    let end = Instant::now() + WAIT * 2;
    let auth = json!({"accessToken": TOKEN, "expiresAt": "2099-01-01T00:00:00Z"}).to_string();
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
        if observed.connections > 4 { return Err(io::Error::other("fixture connection limit")); }
        let Some(bytes) = read_request(&mut stream, &stop, end)? else { continue };
        let request = String::from_utf8_lossy(&bytes);
        let path = request.split_whitespace().nth(1).unwrap_or("");
        let body = match path {
            AUTH => {
                observed.order.push("auth");
                observed.wire_ok &= wire_matches(&bytes, AUTH, true);
                &auth
            }
            LEDGER => {
                observed.order.push("ledger");
                observed.wire_ok &= wire_matches(&bytes, LEDGER, false);
                &ledger
            }
            _ => return Err(io::Error::other("unexpected fixture path")),
        };
        if body.len() > 8192 { return Err(io::Error::other("fixture response limit")); }
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
            Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        stream.write_all(response.as_bytes())?;
    }
    if !stop.load(Ordering::SeqCst) { return Err(io::Error::other("fixture deadline")); }
    Ok(observed)
}

fn ledger(body: Value) -> (Result<CreditSnapshot, String>, Observation) {
    let fixture = Fixture::start(body);
    let transport = Transport::control_for_loopback(fixture.address, Egress::Direct).unwrap();
    let session = Session { active_user: FramerUserId("allocation-fixture-user".into()),
        cookie_header: COOKIE.into() };
    let team = TeamId("allocation-fixture-team".into());
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let result = runtime.block_on(async {
        tokio::time::timeout(WAIT, usage(&transport, &session, &team)).await
    });
    drop(transport);
    drop(runtime);
    let observed = fixture.finish();
    (result.expect("ledger call exceeded fixture bound"), observed.expect("joined fixture failed"))
}

fn fund(amounts: &[f64]) -> Value {
    json!({"grants": amounts.iter().map(|amount|
        json!({"allocatedCredits": amount})).collect::<Vec<_>>()})
}

fn assert_wire(observed: Observation) {
    assert_eq!(observed.connections, 2);
    assert_eq!(observed.order, ["auth", "ledger"]);
    assert!(observed.wire_ok);
}

fn assert_round_trip(snapshot: CreditSnapshot, available: f64,
    funds: u32, grants: u32, allocated: f64) {
    assert_eq!(snapshot.available.to_bits(), available.to_bits());
    assert_eq!(snapshot.funds, funds);
    assert_eq!(snapshot.grants, grants);
    assert!(snapshot.allocated.is_finite());
    assert_eq!(snapshot.allocated, allocated);
    assert_eq!(snapshot.looks_exhausted(), available <= 0.0);
    let encoded = serde_json::to_string(&snapshot).unwrap();
    let restored: CreditSnapshot = serde_json::from_str(&encoded).unwrap();
    assert_eq!(json!(restored), json!(snapshot));
    assert_eq!(restored.available.to_bits(), snapshot.available.to_bits());
    assert_eq!(restored.allocated.to_bits(), snapshot.allocated.to_bits());
}

#[test]
fn positive_allocation_overflow_is_rejected() {
    for funds in [json!([fund(&[LARGE, LARGE])]),
        json!([fund(&[LARGE]), fund(&[LARGE])]),
        json!([fund(&[LARGE, LARGE, -LARGE])])] {
        let (result, observed) = ledger(json!({"availableCredits": 100, "funds": funds}));
        assert_eq!(result.unwrap_err(), INVALID);
        assert_wire(observed);
    }
    for available in [None, Some(json!(null)), Some(json!("100"))] {
        let mut body = json!({"funds": [fund(&[LARGE, LARGE])]});
        if let Some(available) = available { body["availableCredits"] = available; }
        let (result, observed) = ledger(body);
        assert_eq!(result.unwrap_err(), "bad credit ledger response: invalid availableCredits");
        assert_wire(observed);
    }
}

#[test]
fn negative_allocation_overflow_is_rejected() {
    for funds in [json!([fund(&[-LARGE, -LARGE])]),
        json!([fund(&[-LARGE]), fund(&[-LARGE])]),
        json!([fund(&[-LARGE, -LARGE, LARGE])])] {
        let (result, observed) = ledger(json!({"availableCredits": 100, "funds": funds}));
        assert_eq!(result.unwrap_err(), INVALID);
        assert_wire(observed);
    }
}

#[test]
fn finite_allocation_totals_preserve_round_trip() {
    for body in [json!({"availableCredits": 100}),
        json!({"availableCredits": 100, "funds": null}),
        json!({"availableCredits": 100, "funds": "not an array"})] {
        let (result, observed) = ledger(body);
        assert_round_trip(result.unwrap(), 100.0, 0, 0, 0.0);
        assert_wire(observed);
    }
    let defaults = json!([
        {"grants": [{}, {"allocatedCredits": null}, {"allocatedCredits": "500"},
            {"allocatedCredits": false}, {"allocatedCredits": []},
            {"allocatedCredits": {}}, {"allocatedCredits": 125.5, "unknown": true}]},
        fund(&[-25.25, 0.0]), {"grants": null}, {"grants": "not an array"}, null
    ]);
    for (funds, count, grants, allocated) in [
        (json!([]), 0, 0, 0.0),
        (defaults, 5, 9, 100.25),
        (json!([fund(&[-25.5, 13.25])]), 1, 2, -12.25),
        (json!([fund(&[-0.0])]), 1, 1, -0.0),
        (json!([fund(&[LARGE, -LARGE, 1.5])]), 1, 3, 1.5),
        (json!([fund(&[LARGE]), fund(&[-LARGE, LARGE])]), 2, 3, LARGE),
        (json!([fund(&[-LARGE]), fund(&[LARGE, -LARGE])]), 2, 3, -LARGE),
        (json!([fund(&[1e16, 1.0, -1e16])]), 1, 3, 0.0),
    ] {
        let (result, observed) = ledger(json!({
            "availableCredits": 12.5, "funds": funds, "unknown": {"retained": true}
        }));
        assert_round_trip(result.unwrap(), 12.5, count, grants, allocated);
        assert_wire(observed);
    }
    let (result, observed) = ledger(json!({"availableCredits": -0.0, "funds": []}));
    assert_round_trip(result.unwrap(), -0.0, 0, 0, 0.0);
    assert_wire(observed);
}
