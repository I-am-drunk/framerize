//! New application fixture helper. Source only; root owns any execution.
//! Exact synthetic request checking with bounded sockets, buffers and lifetime.

use serde_json::Value;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(super) const STEP: Duration = Duration::from_secs(2);
pub(super) const TICK: Duration = Duration::from_millis(2);
const SERVER_LIFE: Duration = Duration::from_secs(12);
const MAX_PEERS: usize = 4;
const MAX_REQUEST: usize = 64 * 1024;
const MAX_REPLY: usize = 32 * 1024;

#[derive(Clone)]
pub(super) struct Expected {
    pub path: &'static str,
    pub envelope: Value,
    pub reply: Reply,
}

#[derive(Clone)]
pub(super) struct Reply {
    pub status: u16,
    pub content_type: Option<&'static str>,
    pub usage: Option<&'static str>,
    pub body: Vec<u8>,
    pub gate_body: bool,
}

impl Reply {
    pub fn json(body: Value) -> Self {
        Self {
            status: 200,
            content_type: Some("application/json"),
            usage: Some("remaining=7.5&fixture=application"),
            body: serde_json::to_vec(&body).expect("synthetic reply serialization"),
            gate_body: false,
        }
    }

    fn wire(&self) -> (Vec<u8>, Vec<u8>) {
        assert!(self.body.len() <= MAX_REPLY / 4);
        let mut head = format!("HTTP/1.1 {} Fixture\r\n", self.status);
        if let Some(value) = self.content_type {
            assert!(!value.contains('\r') && !value.contains('\n'));
            head.push_str(&format!("Content-Type: {value}\r\n"));
        }
        if let Some(value) = self.usage {
            assert!(!value.contains('\r') && !value.contains('\n'));
            head.push_str(&format!("X-Framer-AI-Usage: {value}\r\n"));
        }
        head.push_str("Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
        let mut body = Vec::new();
        for part in self.body.chunks(7) {
            body.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
            body.extend_from_slice(part);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(b"0\r\n\r\n");
        assert!(head.len() + body.len() <= MAX_REPLY);
        (head.into_bytes(), body)
    }
}

#[derive(Default)]
pub(super) struct Shared {
    pub stop: AtomicBool,
    hold_headers: bool,
    pub release_headers: AtomicBool,
    pub release_body: AtomicBool,
    pub accepted: AtomicUsize,
    pub requests: AtomicUsize,
    pub headers: AtomicUsize,
    pub replies: AtomicUsize,
    pub peer_eof: AtomicUsize,
}

#[derive(Debug)]
pub(super) struct Snapshot {
    pub accepted: usize,
    pub requests: usize,
    pub headers: usize,
    pub replies: usize,
    pub peer_eof: usize,
    pub fault: Option<&'static str>,
}

pub(super) struct Server {
    pub address: SocketAddr,
    pub shared: Arc<Shared>,
    worker: Option<JoinHandle<Snapshot>>,
}

impl Server {
    pub fn new(expected: Vec<Expected>) -> Self {
        Self::create(expected, false)
    }

    pub fn with_held_headers(expected: Vec<Expected>) -> Self {
        Self::create(expected, true)
    }

    fn create(expected: Vec<Expected>, hold_headers: bool) -> Self {
        assert!(expected.len() <= MAX_PEERS);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .expect("application loopback bind");
        listener.set_nonblocking(true).expect("application nonblocking listener");
        let address = listener.local_addr().expect("application loopback address");
        assert!(matches!(address, SocketAddr::V4(a) if *a.ip() == Ipv4Addr::LOCALHOST));
        let shared = Arc::new(Shared { hold_headers, ..Shared::default() });
        let state = shared.clone();
        let worker = thread::spawn(move || serve(listener, expected, state));
        Self { address, shared, worker: Some(worker) }
    }

    fn stop_join(&mut self) -> Snapshot {
        self.shared.stop.store(true, Ordering::Release);
        let until = Instant::now() + STEP;
        while !self.worker.as_ref().expect("owned server worker").is_finished() {
            if Instant::now() >= until {
                // Never detach a worker that may still own a fixture socket.
                std::process::abort();
            }
            thread::sleep(TICK);
        }
        self.worker.take().expect("owned server worker").join().unwrap_or_else(|_| {
            snapshot(&self.shared, Some("application server worker panicked"))
        })
    }

    pub fn finish(mut self) -> Snapshot { self.stop_join() }
}

impl Drop for Server {
    fn drop(&mut self) {
        if self.worker.is_some() { let _ = self.stop_join(); }
    }
}

fn snapshot(shared: &Shared, fault: Option<&'static str>) -> Snapshot {
    Snapshot {
        accepted: shared.accepted.load(Ordering::Acquire),
        requests: shared.requests.load(Ordering::Acquire),
        headers: shared.headers.load(Ordering::Acquire),
        replies: shared.replies.load(Ordering::Acquire),
        peer_eof: shared.peer_eof.load(Ordering::Acquire),
        fault,
    }
}

struct Peer {
    socket: TcpStream,
    expected: Expected,
    request: Vec<u8>,
    length: Option<usize>,
    validated: bool,
    head: Vec<u8>,
    body: Vec<u8>,
    written_head: usize,
    written_body: usize,
    counted_head: bool,
}

fn serve(listener: TcpListener, expected: Vec<Expected>, shared: Arc<Shared>) -> Snapshot {
    let until = Instant::now() + SERVER_LIFE;
    let mut peers = Vec::new();
    let mut fault = None;
    loop {
        let stopping = shared.stop.load(Ordering::Acquire);
        if Instant::now() >= until { fault = Some("application server deadline"); break; }
        for _ in 0..MAX_PEERS {
            match listener.accept() {
                Ok((socket, address)) => {
                    let index = shared.accepted.fetch_add(1, Ordering::AcqRel);
                    if !address.ip().is_loopback() || index >= expected.len()
                        || peers.len() >= MAX_PEERS || socket.set_nonblocking(true).is_err()
                    {
                        let _ = socket.shutdown(Shutdown::Both);
                        fault = Some("unexpected application connection");
                        break;
                    }
                    let expectation = expected[index].clone();
                    let (head, body) = expectation.reply.wire();
                    peers.push(Peer {
                        socket, expected: expectation, request: Vec::new(), length: None,
                        validated: false, head, body, written_head: 0, written_body: 0,
                        counted_head: false,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => { fault = Some("application accept failed"); break; }
            }
        }
        if fault.is_some() { break; }
        // Perform one final accept pass after observing stop. Zero-I/O cases
        // must not hide a connection that was queued just before teardown.
        if stopping { break; }
        for index in (0..peers.len()).rev() {
            match peers[index].advance(&shared) {
                Ok(true) => {}
                Ok(false) => { peers.swap_remove(index); }
                Err(error) => { fault = Some(error); break; }
            }
        }
        if fault.is_some() { break; }
        thread::sleep(TICK);
    }
    drop(peers);
    drop(listener);
    snapshot(&shared, fault)
}

impl Peer {
    fn advance(&mut self, shared: &Shared) -> Result<bool, &'static str> {
        if !self.validated && !self.read_request(shared)? { return Ok(true); }
        if shared.hold_headers && !shared.release_headers.load(Ordering::Acquire) {
            return peer_open(&mut self.socket, shared);
        }
        if self.written_head < self.head.len() {
            if !write_piece(&mut self.socket, &self.head, &mut self.written_head, shared)? {
                return Ok(false);
            }
            if self.written_head < self.head.len() { return Ok(true); }
        }
        if !self.counted_head {
            self.counted_head = true;
            shared.headers.fetch_add(1, Ordering::Release);
        }
        if self.expected.reply.gate_body && !shared.release_body.load(Ordering::Acquire) {
            return peer_open(&mut self.socket, shared);
        }
        if !write_piece(&mut self.socket, &self.body, &mut self.written_body, shared)? {
            return Ok(false);
        }
        if self.written_body < self.body.len() { return Ok(true); }
        shared.replies.fetch_add(1, Ordering::Release);
        let _ = self.socket.shutdown(Shutdown::Write);
        Ok(false)
    }

    fn read_request(&mut self, shared: &Shared) -> Result<bool, &'static str> {
        let mut piece = [0u8; 1024];
        match self.socket.read(&mut piece) {
            Ok(0) => return Err("application request closed early"),
            Ok(count) => {
                if count > MAX_REQUEST.saturating_sub(self.request.len()) {
                    return Err("application request exceeded fixture bound");
                }
                self.request.extend_from_slice(&piece[..count]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(_) => return Err("application request read failed"),
        }
        if self.length.is_none() {
            self.length = request_length(&self.request, self.expected.path)?;
        }
        let Some(length) = self.length else { return Ok(false); };
        if self.request.len() < length { return Ok(false); }
        if self.request.len() != length { return Err("application request extra bytes"); }
        let end = header_end(&self.request).ok_or("application request header")?;
        let head = std::str::from_utf8(&self.request[..end])
            .map_err(|_| "application request header encoding")?;
        let body: Value = serde_json::from_slice(&self.request[end..])
            .map_err(|_| "application request JSON")?;
        if header_value(head, "cookie")? != "session=fixture-application"
            || header_value(head, "authorization")? != "Bearer fixture-application-bearer"
            || header_value(head, "origin")? != "https://framer.com"
            || header_value(head, "referer")? != "https://framer.com/"
            || header_value(head, "user-agent")? != "Framerize/0.1"
            || body != self.expected.envelope
        {
            return Err("application request differs from independent expectation");
        }
        self.validated = true;
        self.request.clear();
        shared.requests.fetch_add(1, Ordering::Release);
        Ok(true)
    }
}

fn write_piece(
    socket: &mut TcpStream, data: &[u8], written: &mut usize, shared: &Shared,
) -> Result<bool, &'static str> {
    let end = (*written + 256).min(data.len());
    match socket.write(&data[*written..end]) {
        Ok(0) => Err("application response zero write"),
        Ok(count) => { *written += count; Ok(true) }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(true),
        Err(e) if matches!(e.kind(), std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted) => {
            shared.peer_eof.fetch_add(1, Ordering::Release);
            Ok(false)
        }
        Err(_) => Err("application response write failed"),
    }
}

fn peer_open(socket: &mut TcpStream, shared: &Shared) -> Result<bool, &'static str> {
    let mut extra = [0u8; 1];
    match socket.read(&mut extra) {
        Ok(0) => {
            shared.peer_eof.fetch_add(1, Ordering::Release);
            Ok(false)
        }
        Ok(_) => Err("extra bytes after application request"),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(true),
        Err(e) if matches!(e.kind(), std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted) => {
            shared.peer_eof.fetch_add(1, Ordering::Release);
            Ok(false)
        }
        Err(_) => Err("application peer-close read failed"),
    }
}

fn header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|part| part == b"\r\n\r\n").map(|n| n + 4)
}

fn header_value<'a>(head: &'a str, name: &str) -> Result<&'a str, &'static str> {
    let mut values = head.split("\r\n").skip(1).filter_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then_some(value.trim())
    });
    let value = values.next().ok_or("missing application fixture header")?;
    if values.next().is_some() { return Err("duplicate application fixture header"); }
    Ok(value)
}

fn request_length(bytes: &[u8], path: &str) -> Result<Option<usize>, &'static str> {
    let Some(end) = header_end(bytes) else { return Ok(None); };
    let head = std::str::from_utf8(&bytes[..end]).map_err(|_| "application header encoding")?;
    if !head.starts_with(&format!("POST {path} HTTP/1.1\r\n"))
        || head.to_ascii_lowercase().contains("\r\ntransfer-encoding:")
    { return Err("unexpected application route or transfer coding"); }
    let length = header_value(head, "content-length")?.parse::<usize>()
        .map_err(|_| "application request length")?;
    let total = end.checked_add(length).filter(|n| *n <= MAX_REQUEST)
        .ok_or("application request too large")?;
    Ok(Some(total))
}
