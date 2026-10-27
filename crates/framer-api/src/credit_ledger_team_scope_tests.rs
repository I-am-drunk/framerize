//! Inert team identity checks through public usage, with bounded, joined peers.
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
const WIRE_LIMIT: usize = 8192;
const AUTH: &str = "/auth/web/access-token";
const TEAM: &str = "01234567-89ab-4cde-8fab-0123456789ab";
const FOREIGN: &str = "01234567-89ab-4cde-8fab-0123456789ac";
const COOKIE: &str = "ledger-team=fixture-cookie";
const TOKEN: &str = "ledger-team-fixture-token";
const INVALID: &str = "bad credit ledger response: incompatible teamId";

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
    fn start(team: &str, ledger: Value) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let path = format!("/web/teams/{team}/ai-credits");
        let worker = thread::spawn(move || serve(listener, signal, path, ledger.to_string()));
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
        let count = match stream.read(&mut buffer) {
            Ok(0) => return Ok(None),
            Ok(n) => n,
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock
                | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => continue,
            Err(e) => return Err(e),
        };
        if bytes.len() + count > WIRE_LIMIT {
            return Err(io::Error::other("fixture input limit"));
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.windows(4).any(|part| part == b"\r\n\r\n") { return Ok(Some(bytes)); }
    }
    Ok(None)
}

fn wire_matches(bytes: &[u8], path: &str, cookie: bool) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let headers: Vec<_> = text.lines().filter_map(|line| line.split_once(':')).collect();
    let header = |name: &str| headers.iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim());
    let bearer = format!("Bearer {TOKEN}");
    let authorization = if cookie {
        header("cookie") == Some(COOKIE) && header("authorization").is_none()
    } else {
        header("authorization") == Some(bearer.as_str()) && header("cookie").is_none()
    };
    text.starts_with(&format!("GET {path} HTTP/1.1\r\n"))
        && header("x-requested-by") == Some("Framer")
        && header("user-agent") == Some("Framerize/0.1")
        && authorization
}

fn serve(listener: TcpListener, stop: Arc<AtomicBool>, ledger_path: String, ledger: String)
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
        let body = if path == AUTH {
            observed.order.push("auth");
            observed.wire_ok &= wire_matches(&bytes, AUTH, true);
            &auth
        } else if path == ledger_path {
            observed.order.push("ledger");
            observed.wire_ok &= wire_matches(&bytes, &ledger_path, false);
            &ledger
        } else {
            return Err(io::Error::other("unexpected fixture path"));
        };
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
            Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        if response.len() > WIRE_LIMIT { return Err(io::Error::other("fixture response limit")); }
        stream.write_all(response.as_bytes())?;
    }
    if !stop.load(Ordering::SeqCst) { return Err(io::Error::other("fixture deadline")); }
    Ok(observed)
}

fn ledger(team: &str, body: Value) -> (Result<CreditSnapshot, String>, Observation) {
    let fixture = Fixture::start(team, body);
    let transport = Transport::control_for_loopback(fixture.address, Egress::Direct).unwrap();
    let session = Session { active_user: FramerUserId("ledger-team-fixture-user".into()),
        cookie_header: COOKIE.into() };
    let team = TeamId(team.into());
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let result = runtime.block_on(async {
        tokio::time::timeout(WAIT, usage(&transport, &session, &team)).await
    });
    drop(transport);
    drop(runtime);
    let observed = fixture.finish();
    (result.expect("ledger call exceeded fixture bound"), observed.expect("joined fixture failed"))
}

fn assert_wire(observed: Observation) {
    assert_eq!(observed.connections, 2);
    assert_eq!(observed.order, ["auth", "ledger"]);
    assert!(observed.wire_ok);
}

fn assert_snapshot(snapshot: CreditSnapshot, expected: (f64, u32, u32, f64)) {
    assert_eq!(snapshot.available.to_bits(), expected.0.to_bits());
    assert_eq!(snapshot.funds, expected.1);
    assert_eq!(snapshot.grants, expected.2);
    assert_eq!(snapshot.allocated.to_bits(), expected.3.to_bits());
    assert_eq!(snapshot.looks_exhausted(), expected.0 <= 0.0);
}

fn detailed() -> Value {
    json!({"availableCredits": 12.5, "funds": [
        {"grants": [{"allocatedCredits": 100.5}, {"allocatedCredits": -0.25}, {}]},
        {"grants": null}, {"unknown": true},
        {"grants": [{"allocatedCredits": null}, {"allocatedCredits": "8"},
            {"allocatedCredits": false}]}
    ], "unknown": {"teamId": "ignored-nested-team"}})
}

#[test]
fn foreign_team_id_is_rejected() {
    for response_team in [FOREIGN.to_owned(), TEAM.to_ascii_uppercase(), format!("{TEAM} ")] {
        let mut body = detailed();
        body["teamId"] = json!(response_team);
        let (result, observed) = ledger(TEAM, body);
        assert_eq!(result.unwrap_err(), INVALID);
        assert_wire(observed);
    }
}

#[test]
fn malformed_team_id_is_rejected() {
    for response_team in [Value::Null, json!(42), json!(true), json!([]), json!({}), json!("")] {
        let mut body = detailed();
        body["teamId"] = response_team;
        let (result, observed) = ledger(TEAM, body);
        assert_eq!(result.unwrap_err(), INVALID);
        assert_wire(observed);
    }
}

#[test]
fn absent_and_matching_team_ids_preserve_snapshot() {
    for team in [TEAM, "arbitrary-full-team-id", ""] {
        for response_team in [None, Some(team)] {
            for (mut body, expected) in [
                (detailed(), (12.5, 4, 6, 100.25)),
                (json!({"availableCredits": -0.0}), (-0.0, 0, 0, 0.0)),
                (json!({"availableCredits": -2.75, "funds": null}), (-2.75, 0, 0, 0.0)),
            ] {
                if let Some(response_team) = response_team {
                    body["teamId"] = json!(response_team);
                }
                let (result, observed) = ledger(team, body);
                assert_snapshot(result.unwrap(), expected);
                assert_wire(observed);
            }
        }
    }
}

fn overflowing_funds(amount: f64) -> Value {
    json!([{"grants": [{"allocatedCredits": amount}, {"allocatedCredits": amount}]}])
}

#[test]
fn numeric_errors_precede_incompatible_team_id() {
    for available in [None, Some(Value::Null), Some(json!("12.5"))] {
        for overflow in [false, true] {
            let mut body = json!({"teamId": FOREIGN});
            if let Some(available) = &available { body["availableCredits"] = available.clone(); }
            if overflow { body["funds"] = overflowing_funds(1e308); }
            let (result, observed) = ledger(TEAM, body);
            assert_eq!(result.unwrap_err(), "bad credit ledger response: invalid availableCredits");
            assert_wire(observed);
        }
    }
    for amount in [1e308, -1e308] {
        let body = json!({"teamId": FOREIGN, "availableCredits": 12.5,
            "funds": overflowing_funds(amount)});
        let (result, observed) = ledger(TEAM, body);
        assert_eq!(result.unwrap_err(), "bad credit ledger response: invalid allocatedCredits total");
        assert_wire(observed);
    }
}
