use super::project_client_builder;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const SHORT: Duration = Duration::from_millis(100);
const LONG: Duration = Duration::from_secs(2);
const LIMIT: Duration = Duration::from_secs(3);
const SERVER_LIMIT: Duration = Duration::from_secs(6);
const PATH: &str = "/projects/new?team=fixture-space";
const LOCATION: &str = "/projects/fixture-project-0123456789";
const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:152.0) Gecko/20100101 Firefox/152.0";

#[derive(Clone, Copy)]
enum Reply { StalledHeaders, StalledTls, Redirect }

#[derive(Default)]
struct Observation {
    connections: usize,
    requests: usize,
    tls_bytes: usize,
    wire_ok: bool,
}

struct Fixture {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    request_seen: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<Observation>>>,
}

impl Fixture {
    fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let request_seen = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let seen = request_seen.clone();
        let worker = thread::spawn(move || serve(listener, signal, seen, reply));
        Self { address, stop, request_seen, worker: Some(worker) }
    }

    fn finish(mut self) -> Observation {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().expect("fixture worker panicked")
            .expect("fixture socket failed")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Every exit, including a failed assertion, joins this fixture owner.
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn read_input(stream: &mut TcpStream, stop: &AtomicBool, end: Instant,
    tls: bool) -> io::Result<Option<Vec<u8>>> {
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
        if tls || bytes.windows(4).any(|part| part == b"\r\n\r\n") {
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

fn wire_matches(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let lower = text.to_ascii_lowercase();
    text.starts_with(&format!("GET {PATH} HTTP/1.1\r\n"))
        && lower.contains("\r\ncookie: fixture-cookie=opaque\r\n")
        && lower.contains(&format!("\r\nuser-agent: {}\r\n", USER_AGENT.to_ascii_lowercase()))
}

fn serve(listener: TcpListener, stop: Arc<AtomicBool>, seen: Arc<AtomicBool>,
    reply: Reply) -> io::Result<Observation> {
    let end = Instant::now() + SERVER_LIMIT;
    let mut observed = Observation { wire_ok: true, ..Observation::default() };
    let mut held = Vec::new();
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
        let tls = matches!(reply, Reply::StalledTls);
        let Some(bytes) = read_input(&mut stream, &stop, end, tls)? else { continue };
        if tls {
            observed.tls_bytes += bytes.len();
            observed.wire_ok &= bytes.first() == Some(&22);
        } else {
            observed.requests += 1;
            observed.wire_ok &= wire_matches(&bytes);
            seen.store(true, Ordering::SeqCst);
        }
        if matches!(reply, Reply::Redirect) {
            let response = format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {LOCATION}\r\nContent-Length: 1024\r\n\r\nx");
            stream.write_all(response.as_bytes())?;
        }
        held.push(stream);
    }
    Ok(observed)
}

fn client(connect: Duration, total: Duration) -> reqwest::Client {
    project_client_builder(connect, total).no_proxy().build().unwrap()
}

fn request(client: &reqwest::Client, address: SocketAddr, tls: bool)
    -> reqwest::RequestBuilder {
    assert!(address.ip().is_loopback());
    let scheme = if tls { "https" } else { "http" };
    client.get(format!("{scheme}://{address}{PATH}"))
        .header("cookie", "fixture-cookie=opaque")
        .header("user-agent", USER_AGENT)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

fn assert_one(observed: &Observation, tls: bool) {
    assert_eq!(observed.connections, 1);
    assert!(observed.wire_ok, "unexpected request or TLS handshake");
    if tls {
        assert_eq!(observed.requests, 0);
        assert!(observed.tls_bytes > 0);
    } else {
        assert_eq!(observed.requests, 1);
        assert_eq!(observed.tls_bytes, 0);
    }
}

fn stalled(tls: bool) {
    let reply = if tls { Reply::StalledTls } else { Reply::StalledHeaders };
    let fixture = Fixture::start(reply);
    let client = if tls { client(SHORT, LONG) } else { client(LONG, SHORT) };
    let runtime = runtime();
    let started = Instant::now();
    let result = runtime.block_on(async {
        tokio::time::timeout(LIMIT, async {
            request(&client, fixture.address, tls).send().await.map(|r| r.status())
        }).await
    });
    let elapsed = started.elapsed();
    drop(client);
    drop(runtime);
    let observed = fixture.finish();
    let error = result.expect("project call exceeded outer fixture bound")
        .expect_err("stalled peer unexpectedly returned headers");
    assert!(error.is_timeout(), "expected reqwest deadline error");
    assert_eq!(error.is_connect(), tls);
    assert!(elapsed >= SHORT && elapsed < LONG, "wrong deadline: {elapsed:?}");
    assert_one(&observed, tls);
}

#[test]
fn stalled_response_headers_respect_total_deadline() { stalled(false); }

#[test]
fn stalled_tls_handshake_respects_connect_deadline() { stalled(true); }

#[test]
fn redirect_headers_return_without_following_or_body() {
    let fixture = Fixture::start(Reply::Redirect);
    let client = client(LONG, LONG);
    let runtime = runtime();
    let started = Instant::now();
    let result = runtime.block_on(async {
        tokio::time::timeout(LIMIT, async {
            request(&client, fixture.address, false).send().await.map(|r| {
                (r.status(), r.headers().get(reqwest::header::LOCATION).cloned())
            })
        }).await
    });
    let elapsed = started.elapsed();
    drop(client);
    drop(runtime);
    let observed = fixture.finish();
    let (status, location) = result.expect("redirect exceeded fixture bound")
        .expect("redirect headers failed");
    assert_eq!(status, reqwest::StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(location.unwrap().to_str().unwrap(), LOCATION);
    assert!(elapsed < LONG, "waited for the incomplete body");
    assert_one(&observed, false);
}

#[test]
fn cancelled_project_request_joins_fixture() {
    let fixture = Fixture::start(Reply::StalledHeaders);
    let client = client(LONG, LONG);
    let runtime = runtime();
    let result = runtime.block_on(async {
        let in_flight = request(&client, fixture.address, false).send();
        tokio::pin!(in_flight);
        tokio::time::timeout(LIMIT, async {
            tokio::select! {
                result = &mut in_flight => { drop(result); false }
                _ = async {
                    while !fixture.request_seen.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                } => true,
            }
        }).await
        // The owned request future is dropped before its client and runtime.
    });
    drop(client);
    drop(runtime);
    let observed = fixture.finish();
    assert!(result.expect("cancellation barrier exceeded fixture bound"),
        "request completed before caller cancellation");
    assert_one(&observed, false);
}
