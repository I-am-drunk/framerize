//! Inert HTTP schedules for the existing cached token entry points.
use super::{cached_mint, web_access_token, AiToken};
use crate::{session_auth::Egress, transport::Transport, SessionAuthorization};
use framer_core::{port::Session, FramerUserId};
use std::{future::{poll_fn, Future}, io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}},
    task::Poll, thread::{self, JoinHandle}, time::{Duration, Instant}};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(3);
const WEB: &str = "/auth/web/access-token";
const OTHER: &str = "/auth/fixture/access-token";
type Outcome = Result<String, String>;

#[derive(Default)]
struct Control {
    requests: AtomicUsize,
    release: AtomicBool,
    stop: AtomicBool,
    cancelled_first: AtomicBool,
}

struct Fixture {
    address: SocketAddr,
    control: Arc<Control>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl Fixture {
    fn start(fail_first: bool) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let control = Arc::new(Control::default());
        let signal = control.clone();
        let worker = thread::spawn(move || serve(listener, &signal, fail_first));
        Self { address, control, worker: Some(worker) }
    }

    fn transport(&self) -> Transport {
        Transport::control_for_loopback(self.address, Egress::Direct).unwrap()
    }

    fn finish(mut self) -> io::Result<usize> {
        self.control.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join()
            .map_err(|_| io::Error::other("fixture worker panicked"))??;
        Ok(self.control.requests.load(Ordering::SeqCst))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

struct Peer { socket: TcpStream, headers: Vec<u8>, number: Option<usize> }

fn step(peer: &mut Peer, control: &Control, fail_first: bool) -> io::Result<bool> {
    if peer.number.is_none() {
        let mut buffer = [0; 1024];
        match peer.socket.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(n) => peer.headers.extend_from_slice(&buffer[..n]),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted)
                => return Ok(false),
            Err(e) if e.kind() == io::ErrorKind::ConnectionReset => return Ok(true),
            Err(e) => return Err(e),
        }
        if peer.headers.len() > 8192 {
            return Err(io::Error::other("fixture header limit"));
        }
        if !peer.headers.windows(4).any(|part| part == b"\r\n\r\n") { return Ok(false); }
        peer.number = Some(control.requests.fetch_add(1, Ordering::SeqCst) + 1);
    }
    let number = peer.number.unwrap();
    if number == 1 && !control.release.load(Ordering::SeqCst) { return Ok(false); }
    reply(peer, control, fail_first, number)?;
    Ok(true)
}

fn reply(peer: &mut Peer, control: &Control, fail_first: bool, number: usize)
    -> io::Result<()> {
    let body = if fail_first && number == 1 { "{}".to_owned() } else {
        serde_json::json!({"accessToken": format!("fixture-mint-{number}"),
            "expiresAt": "2099-01-01T00:00:00Z"}).to_string()
    };
    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
        Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    peer.socket.set_nonblocking(false)?;
    peer.socket.set_write_timeout(Some(Duration::from_millis(100)))?;
    match peer.socket.write_all(response.as_bytes()) {
        Err(e) if number == 1 && control.cancelled_first.load(Ordering::SeqCst)
            && matches!(e.kind(), io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted | io::ErrorKind::NotConnected) => Ok(()),
        result => result,
    }
}

fn serve(listener: TcpListener, control: &Control, fail_first: bool) -> io::Result<()> {
    let deadline = Instant::now() + WAIT * 4;
    let mut peers = Vec::new();
    let mut accepted = 0;
    while !control.stop.load(Ordering::SeqCst) {
        if Instant::now() >= deadline { return Err(io::Error::other("fixture deadline")); }
        match listener.accept() {
            Ok((socket, _)) => {
                accepted += 1;
                if accepted > 16 { return Err(io::Error::other("fixture connection limit")); }
                socket.set_nonblocking(true)?;
                peers.push(Peer { socket, headers: Vec::new(), number: None });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => (),
            Err(e) => return Err(e),
        }
        let mut index = 0;
        while index < peers.len() {
            if step(&mut peers[index], control, fail_first)? { peers.swap_remove(index); }
            else { index += 1; }
        }
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

fn session() -> Session {
    Session { active_user: FramerUserId("fixture-mint-user".into()),
        cookie_header: format!("auth-mint-concurrency={}", uuid::Uuid::new_v4()) }
}

async fn token(transport: &Transport, session: &Session, path: &str) -> Outcome {
    let result = if path == WEB { web_access_token(transport, session).await }
        else { cached_mint(transport, session, path).await };
    result.map(|token| token.bearer)
}

fn cached(transport: &Transport, session: &Session, path: &str) -> Option<String> {
    transport.authorization.sessions.lock().unwrap()
        .get(&(session.cookie_header.clone(), path.to_owned()))
        .filter(|token| token.usable_for(session)).map(|token| token.bearer.clone())
}

async fn until(mut predicate: impl FnMut() -> bool) -> bool {
    timeout(WAIT, async {
        while !predicate() { tokio::time::sleep(Duration::from_millis(1)).await; }
    }).await.is_ok()
}

async fn first_request(control: &Control) -> bool {
    until(|| control.requests.load(Ordering::SeqCst) > 0).await
}

async fn counted<F: Future>(future: F, counter: &AtomicUsize) -> F::Output {
    tokio::pin!(future);
    let mut first = true;
    poll_fn(|cx| {
        let result = future.as_mut().poll(cx);
        if first {
            first = false;
            counter.fetch_add(1, Ordering::SeqCst);
        }
        result
    }).await
}

async fn poll_once<F: Future>(mut future: std::pin::Pin<&mut F>) -> Option<F::Output> {
    poll_fn(|cx| Poll::Ready(match future.as_mut().poll(cx) {
        Poll::Ready(result) => Some(result),
        Poll::Pending => None,
    })).await
}

async fn same_key_case(expired: bool) {
    let fixture = Fixture::start(false);
    let transport = fixture.transport();
    let session = session();
    if expired {
        transport.authorization.sessions.lock().unwrap().insert(
            (session.cookie_header.clone(), WEB.to_owned()),
            AiToken::for_session(&session, "expired-fixture".into(), 0));
    }
    let control = fixture.control.clone();
    let polled = AtomicUsize::new(0);
    let outcomes = timeout(WAIT * 3, async {
        let waiting = async {
            let entered = first_request(&control).await;
            SessionAuthorization(transport.authorization.clone()).invalidate(&session);
            let (results, ready) = tokio::join!(
                futures::future::join_all((0..3).map(|_| counted(
                    token(&transport, &session, WEB), &polled))),
                async {
                    let ready = until(|| polled.load(Ordering::SeqCst) == 3).await;
                    control.release.store(true, Ordering::SeqCst);
                    ready
                });
            (entered, results, ready)
        };
        tokio::join!(token(&transport, &session, WEB), waiting)
    }).await;
    control.release.store(true, Ordering::SeqCst);
    let requests = fixture.finish();
    let (leader, (entered, waiters, ready)) = outcomes.expect("joined caller deadline");
    assert!(entered && ready, "all callers must reach the held response");
    assert_eq!(requests.unwrap(), 1, "expired={expired}");
    for result in std::iter::once(leader).chain(waiters) {
        assert_eq!(result, Ok("fixture-mint-1".into()));
    }
    assert_eq!(cached(&transport, &session, WEB).as_deref(), Some("fixture-mint-1"));
}

#[tokio::test]
async fn concurrent_misses_share_successful_mint() {
    for expired in [false, true] { same_key_case(expired).await; }
}

#[derive(Clone, Copy)]
enum Split { Cookie, Path, Runtime }

async fn independent_case(split: Split) {
    let fixture = Fixture::start(false);
    let first = fixture.transport();
    let mut second = fixture.transport();
    if !matches!(split, Split::Runtime) { second.authorization = first.authorization.clone(); }
    let a = session();
    let mut b = a.clone();
    if matches!(split, Split::Cookie) { b.cookie_header.push_str("-other"); }
    let path = if matches!(split, Split::Path) { OTHER } else { WEB };
    let control = fixture.control.clone();
    let outcomes = timeout(WAIT * 3, async {
        tokio::join!(token(&first, &a, WEB), async {
            let entered = first_request(&control).await;
            let result = timeout(WAIT, token(&second, &b, path)).await;
            control.release.store(true, Ordering::SeqCst);
            (entered, result)
        })
    }).await;
    control.release.store(true, Ordering::SeqCst);
    let requests = fixture.finish();
    let (first_result, (entered, second_result)) = outcomes.expect("joined caller deadline");
    assert!(entered);
    assert_eq!(second_result.expect("independent mint must finish while first is held"),
        Ok("fixture-mint-2".into()));
    assert_eq!(first_result, Ok("fixture-mint-1".into()));
    assert_eq!(requests.unwrap(), 2);
    assert_eq!(cached(&first, &a, WEB).as_deref(), Some("fixture-mint-1"));
    assert_eq!(cached(&second, &b, path).as_deref(), Some("fixture-mint-2"));
}

#[tokio::test]
async fn distinct_cache_keys_mint_independently() {
    for split in [Split::Cookie, Split::Path] { independent_case(split).await; }
}

#[tokio::test]
async fn independent_authorization_runtimes_mint_independently() {
    independent_case(Split::Runtime).await;
}

#[tokio::test]
async fn failed_mint_releases_key_without_caching_error() {
    let fixture = Fixture::start(true);
    let transport = fixture.transport();
    let session = session();
    let control = fixture.control.clone();
    let polled = AtomicUsize::new(0);
    let outcomes = timeout(WAIT * 3, async {
        tokio::join!(token(&transport, &session, WEB), async {
            let entered = first_request(&control).await;
            let (result, ready) = tokio::join!(
                counted(token(&transport, &session, WEB), &polled),
                async {
                    let ready = until(|| polled.load(Ordering::SeqCst) == 1).await;
                    control.release.store(true, Ordering::SeqCst);
                    ready
                });
            (entered, result, ready)
        })
    }).await;
    control.release.store(true, Ordering::SeqCst);
    let cached_result = timeout(WAIT, token(&transport, &session, WEB)).await;
    let requests = fixture.finish();
    let (failed, (entered, retried, ready)) = outcomes.expect("joined caller deadline");
    assert!(entered && ready);
    assert_eq!(failed, Err("bad access-token response".into()));
    assert_eq!(retried, Ok("fixture-mint-2".into()));
    assert_eq!(cached_result.unwrap(), Ok("fixture-mint-2".into()));
    assert_eq!(cached(&transport, &session, WEB).as_deref(), Some("fixture-mint-2"));
    assert_eq!(requests.unwrap(), 2);
}

#[tokio::test]
async fn cancelled_waiter_does_not_mint() {
    let fixture = Fixture::start(false);
    let transport = fixture.transport();
    let session = session();
    let control = fixture.control.clone();
    let outcomes = timeout(WAIT * 3, async {
        tokio::join!(token(&transport, &session, WEB), async {
            let entered = first_request(&control).await;
            let mut waiter = Box::pin(token(&transport, &session, WEB));
            let early = poll_once(waiter.as_mut()).await;
            drop(waiter);
            control.release.store(true, Ordering::SeqCst);
            (entered, early)
        })
    }).await;
    control.release.store(true, Ordering::SeqCst);
    let requests = fixture.finish();
    let (leader, (entered, early)) = outcomes.expect("joined caller deadline");
    assert!(entered);
    assert!(early.is_none(), "waiter must be pending before cancellation");
    assert_eq!(leader, Ok("fixture-mint-1".into()));
    assert_eq!(cached(&transport, &session, WEB).as_deref(), Some("fixture-mint-1"));
    assert_eq!(requests.unwrap(), 1);
}

#[tokio::test]
async fn cancelled_leader_releases_key_for_waiter() {
    let fixture = Fixture::start(false);
    let transport = fixture.transport();
    let session = session();
    let control = fixture.control.clone();
    let mut leader = Box::pin(token(&transport, &session, WEB));
    let (entered, early_leader) = tokio::select! {
        result = leader.as_mut() => (false, Some(result)),
        entered = first_request(&control) => (entered, None),
    };
    let mut waiter = Box::pin(token(&transport, &session, WEB));
    let early_waiter = poll_once(waiter.as_mut()).await;
    control.cancelled_first.store(true, Ordering::SeqCst);
    drop(leader);
    let result = match &early_waiter {
        Some(result) => Ok(result.clone()),
        None => timeout(WAIT, waiter.as_mut()).await,
    };
    drop(waiter);
    // Neither the retry nor the cache hit needs the abandoned response.
    let cached_result = timeout(WAIT, token(&transport, &session, WEB)).await;
    control.release.store(true, Ordering::SeqCst);
    let requests = fixture.finish();
    assert!(entered && early_leader.is_none());
    assert!(early_waiter.is_none());
    assert_eq!(result.expect("cancelled leader must release its key"),
        Ok("fixture-mint-2".into()));
    assert_eq!(cached_result.unwrap(), Ok("fixture-mint-2".into()));
    assert_eq!(cached(&transport, &session, WEB).as_deref(), Some("fixture-mint-2"));
    assert_eq!(requests.unwrap(), 2);
}
