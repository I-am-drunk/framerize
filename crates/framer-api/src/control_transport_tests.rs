use super::{normal_client_builder, Auth, Transport};
use framer_core::{port::Session, FramerUserId};
use serde_json::{json, Value};
use std::future::Future;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{atomic::{AtomicBool, AtomicUsize, Ordering}, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CONNECT: Duration = Duration::from_millis(100);
const TOTAL: Duration = Duration::from_millis(250);
const LIMIT: Duration = Duration::from_secs(2);
const PATH: &str = "/fixture?redaction_probe=inert";
const BODY: &str = "{\"fixture\":true}";

#[derive(Clone, Copy)]
enum Credential { None, Cookie, Bearer }

impl Credential {
    fn auth(self, session: &Session) -> Auth<'_> {
        match self {
            Self::None => Auth::None,
            Self::Cookie => Auth::Cookie(session),
            Self::Bearer => Auth::Bearer("fixture-bearer"),
        }
    }
}

#[derive(Clone, Copy)]
struct Wire { method: &'static str, path: &'static str, auth: Credential }

impl Wire {
    fn matches(self, request: &[u8]) -> bool {
        let request = String::from_utf8_lossy(request);
        let Some((headers, body)) = request.split_once("\r\n\r\n") else { return false };
        let lower = format!("{headers}\r\n").to_ascii_lowercase();
        let bearer = lower.contains("\r\nauthorization: bearer fixture-bearer\r\n");
        let cookie = lower.contains("\r\ncookie: fixture-cookie=opaque\r\n");
        let no_bearer = !lower.contains("\r\nauthorization:");
        let no_cookie = !lower.contains("\r\ncookie:");
        let auth_ok = match self.auth {
            Credential::None => no_bearer && no_cookie,
            Credential::Cookie => cookie && no_bearer,
            Credential::Bearer => bearer && no_cookie,
        };
        headers.starts_with(&format!("{} {} HTTP/1.1\r\n", self.method, self.path))
            && lower.contains("\r\nuser-agent: framerize/0.1\r\n")
            && lower.contains("\r\nx-requested-by: framer\r\n")
            && auth_ok
            && if self.method == "POST" {
                body == BODY && lower.contains("\r\ncontent-type: application/json\r\n")
            } else { body.is_empty() }
    }
}

#[derive(Clone, Copy)]
enum Reply { StalledHeaders, StalledBody, Complete(u16, &'static str) }

#[derive(Default)]
struct Observation { connections: usize, requests: usize, wire_ok: bool }

struct Fixture {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    worker: Option<JoinHandle<io::Result<Observation>>>,
}

impl Fixture {
    fn start(reply: Reply, wire: Wire) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let signal = stop.clone();
        let counter = requests.clone();
        let worker = thread::spawn(move || serve(listener, signal, counter, reply, wire));
        Self { address, stop, requests, worker: Some(worker) }
    }

    fn finish(mut self) -> Observation {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().expect("fixture worker panicked")
            .expect("fixture socket failed")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Panic paths also signal and join the owned worker; no detached server.
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

fn read_request(stream: &mut TcpStream, stop: &AtomicBool) -> io::Result<Option<Vec<u8>>> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_millis(10)))?;
    let end = Instant::now() + LIMIT;
    let mut bytes = Vec::new();
    while !stop.load(Ordering::SeqCst) && Instant::now() < end {
        let mut buffer = [0; 1024];
        match stream.read(&mut buffer) {
            Ok(0) => return Ok(None),
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => continue,
            Err(e) => return Err(e),
        }
        if bytes.len() > 8192 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "fixture request exceeds limit"));
        }
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers.lines().filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse::<usize>())
                .transpose().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "fixture length"))?
                .unwrap_or(0);
            if length > 8192 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "fixture body exceeds limit"));
            }
            if bytes.len() >= end + 4 + length { return Ok(Some(bytes)); }
        }
    }
    Ok(None)
}

fn serve(listener: TcpListener, stop: Arc<AtomicBool>, requests: Arc<AtomicUsize>,
    reply: Reply, wire: Wire) -> io::Result<Observation> {
    let mut observed = Observation { wire_ok: true, ..Observation::default() };
    let mut held = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        let (mut stream, _) = match listener.accept() {
            Ok(connection) => connection,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
                continue;
            }
            Err(e) => return Err(e),
        };
        observed.connections += 1;
        if observed.connections > 8 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "fixture connection limit"));
        }
        let Some(bytes) = read_request(&mut stream, &stop)? else { continue };
        observed.requests += 1;
        observed.wire_ok &= wire.matches(&bytes);
        requests.fetch_add(1, Ordering::SeqCst);
        stream.set_write_timeout(Some(Duration::from_millis(100)))?;
        match reply {
            Reply::StalledHeaders => held.push(stream),
            Reply::StalledBody => {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1024\r\n\r\n{")?;
                held.push(stream);
            }
            Reply::Complete(status, body) => {
                write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
            }
        }
    }
    Ok(observed)
}

fn client(address: SocketAddr) -> Transport {
    assert!(address.ip().is_loopback());
    Transport {
        http: normal_client_builder(CONNECT, TOTAL).no_proxy().build().unwrap(),
        base: format!("http://{address}"),
    }
}

fn session() -> Session {
    Session { active_user: FramerUserId("fixture-user".into()),
        cookie_header: "fixture-cookie=opaque".into() }
}

fn run<F: Future>(future: F) -> F::Output {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        tokio::time::timeout(LIMIT, future).await.expect("control call exceeded fixture bound")
    })
}

async fn send(client: &Transport, wire: Wire, session: &Session) -> Result<reqwest::Response, String> {
    let auth = wire.auth.auth(session);
    if wire.method == "GET" { client.get(wire.path, auth).await }
    else { client.post(wire.path, auth, json!({"fixture": true})).await }
}

fn assert_one(observed: Observation) {
    assert_eq!(observed.connections, 1);
    assert_eq!(observed.requests, 1);
    assert!(observed.wire_ok, "request method, auth, marker or body changed");
}

fn stalled_headers(method: &'static str) {
    let wire = Wire { method, path: PATH, auth: Credential::None };
    let fixture = Fixture::start(Reply::StalledHeaders, wire);
    let client = client(fixture.address);
    let session = session();
    let started = Instant::now();
    let result = run(send(&client, wire, &session));
    let elapsed = started.elapsed();
    drop(client);
    let observed = fixture.finish();
    assert_send_error(&result.unwrap_err(), true, false);
    assert!(elapsed >= TOTAL && elapsed < LIMIT);
    assert_one(observed);
}

#[test]
fn normal_get_stalled_headers_times_out_once() { stalled_headers("GET"); }

#[test]
fn normal_post_stalled_headers_times_out_once() { stalled_headers("POST"); }

#[test]
fn normal_get_stalled_body_times_out_once() {
    let wire = Wire { method: "GET", path: PATH, auth: Credential::None };
    let fixture = Fixture::start(Reply::StalledBody, wire);
    let client = client(fixture.address);
    let started = Instant::now();
    let result = run(async { client.get(PATH, Auth::None).await.unwrap().bytes().await });
    let elapsed = started.elapsed();
    drop(client);
    let observed = fixture.finish();
    assert!(result.unwrap_err().is_timeout());
    assert!(elapsed >= TOTAL && elapsed < LIMIT);
    assert_one(observed);
}

#[test]
fn normal_post_json_stalled_body_times_out_once() {
    let wire = Wire { method: "POST", path: PATH, auth: Credential::None };
    let fixture = Fixture::start(Reply::StalledBody, wire);
    let client = client(fixture.address);
    let started = Instant::now();
    let result = run(client.post_json(PATH, Auth::None, json!({"fixture": true})));
    let elapsed = started.elapsed();
    drop(client);
    let observed = fixture.finish();
    assert!(result.unwrap_err().starts_with("decode "));
    assert!(elapsed >= TOTAL && elapsed < LIMIT);
    assert_one(observed);
}

#[test]
fn normal_refused_connection_is_bounded() {
    for method in ["GET", "POST"] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = client(listener.local_addr().unwrap());
        drop(listener);
        let wire = Wire { method, path: PATH, auth: Credential::None };
        let error = run(send(&client, wire, &session())).unwrap_err();
        assert_send_error(&error, false, true);
    }
}

#[test]
fn normal_json_and_auth_are_preserved() {
    for method in ["GET", "POST"] {
        for auth in [Credential::None, Credential::Cookie, Credential::Bearer] {
            let wire = Wire { method, path: PATH, auth };
            let fixture = Fixture::start(Reply::Complete(200, BODY), wire);
            let client = client(fixture.address);
            let session = session();
            let value = run(async {
                if method == "GET" {
                    send(&client, wire, &session).await.unwrap().json::<Value>().await.unwrap()
                } else {
                    client.post_json(PATH, auth.auth(&session), json!({"fixture": true})).await.unwrap()
                }
            });
            drop(client);
            let observed = fixture.finish();
            assert_eq!(value, json!({"fixture": true}));
            assert_one(observed);
        }
    }
}

#[test]
fn normal_status_policy_is_preserved() {
    let cases = [
        (401, "/auth/ai/access-token", "ignored", "unauthorized"),
        (402, "/auth/ai/access-token", "ignored", "credits-exhausted"),
        (403, "/auth/ai/access-token", "Forbidden", "retryable 403 on /auth/ai/access-token: Forbidden"),
        (403, "/ai/v3/openai/responses/", "Forbidden", "risk-refused"),
        (429, "/auth/ai/access-token", "limited", "retryable 429 on /auth/ai/access-token: limited"),
        (500, "/auth/ai/access-token", "unavailable", "retryable 500 on /auth/ai/access-token: unavailable"),
    ];
    for method in ["GET", "POST"] {
        for (status, path, body, expected) in cases {
            let wire = Wire { method, path, auth: Credential::None };
            let fixture = Fixture::start(Reply::Complete(status, body), wire);
            let client = client(fixture.address);
            let result = run(send(&client, wire, &session()));
            drop(client);
            let observed = fixture.finish();
            assert_eq!(result.unwrap_err(), expected);
            assert_one(observed);
        }
    }
}

#[test]
fn normal_caller_cancellation_joins_fixture() {
    let wire = Wire { method: "GET", path: PATH, auth: Credential::None };
    let fixture = Fixture::start(Reply::StalledHeaders, wire);
    let client = client(fixture.address);
    let session = session();
    run(async {
        let request = Box::pin(send(&client, wire, &session));
        let entered = Box::pin(async {
            while fixture.requests.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        });
        match futures::future::select(request, entered).await {
            futures::future::Either::Right(((), pending)) => drop(pending),
            futures::future::Either::Left(_) => panic!("request finished before caller cancellation"),
        }
    });
    drop(client);
    assert_one(fixture.finish());
}

fn assert_send_error(error: &str, timeout: bool, connect: bool) {
    let flags: Vec<_> = error.strip_prefix("control transport failed: ")
        .expect("missing safe error prefix").split(' ').collect();
    assert_eq!(flags.len(), 5);
    for (flag, name) in flags.iter().zip(["timeout", "connect", "request", "body", "decode"]) {
        let prefix = format!("{name}=");
        assert!(matches!(flag.strip_prefix(prefix.as_str()), Some("true" | "false")));
    }
    assert_eq!(flags[0], format!("timeout={timeout}"));
    assert_eq!(flags[1], format!("connect={connect}"));
    assert_eq!(flags[3], "body=false");
    assert_eq!(flags[4], "decode=false");
}

#[test]
fn normal_send_error_redaction_covers_invalid_urls() {
    for method in ["GET", "POST"] {
        let client = Transport {
            http: normal_client_builder(CONNECT, TOTAL).no_proxy().build().unwrap(),
            base: "http://[redaction-probe=inert".into(),
        };
        let wire = Wire { method, path: PATH, auth: Credential::None };
        let error = run(send(&client, wire, &session())).unwrap_err();
        assert_send_error(&error, false, false);
    }
}

#[test]
fn normal_send_error_redaction_covers_invalid_headers() {
    for method in ["GET", "POST"] {
        let wire = Wire { method, path: PATH, auth: Credential::Bearer };
        let fixture = Fixture::start(Reply::Complete(200, BODY), wire);
        let client = client(fixture.address);
        let auth = Auth::Bearer("fixture-bearer\r\nredaction-probe: inert");
        let error = run(async {
            if method == "GET" { client.get(PATH, auth).await }
            else { client.post(PATH, auth, json!({"fixture": true})).await }
        }).unwrap_err();
        drop(client);
        let observed = fixture.finish();
        assert_send_error(&error, false, false);
        assert_eq!(observed.connections, 0);
        assert_eq!(observed.requests, 0);
    }
}
