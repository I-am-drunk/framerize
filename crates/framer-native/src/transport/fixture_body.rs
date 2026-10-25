//! Test-only child of the pinned real native_http module.
//! Replaces delivery, not NativeByteStream, cancellation or Permit.
use super::*;
use std::collections::VecDeque;
use std::task::Waker;

pub struct Generated {
    pub account: usize,
    pub count: usize,
    pub padding: usize,
    pub cooperate: bool,
}

struct Control {
    chunks: VecDeque<Bytes>,
    eof: bool,
    closed: bool,
    waker: Option<Waker>,
    generated: Option<Generated>,
    next: usize,
    terminal_sent: bool,
    yield_next: bool,
}

struct ProbeInner {
    control: Mutex<Control>,
    polls: AtomicUsize,
    drops: AtomicUsize,
    generated: AtomicUsize,
    max_chunk: AtomicUsize,
    active: Arc<AtomicUsize>,
    stream: Mutex<Option<Weak<Mutex<StreamingState>>>>,
}

#[derive(Clone)]
pub struct Probe(Arc<ProbeInner>);

impl Probe {
    pub fn polls(&self) -> usize { self.0.polls.load(Ordering::SeqCst) }
    pub fn drops(&self) -> usize { self.0.drops.load(Ordering::SeqCst) }
    pub fn active(&self) -> usize { self.0.active.load(Ordering::SeqCst) }
    pub fn generated(&self) -> usize { self.0.generated.load(Ordering::SeqCst) }
    pub fn max_chunk(&self) -> usize { self.0.max_chunk.load(Ordering::SeqCst) }
    pub fn pending_bytes(&self) -> usize {
        lock(&self.0.stream).as_ref().and_then(Weak::upgrade)
            .map(|state| lock(&state).pending.len()).unwrap_or(0)
    }
    pub fn push(&self, bytes: &[u8]) {
        let wake = {
            let mut state = lock(&self.0.control);
            assert!(!state.closed && state.generated.is_none());
            state.chunks.push_back(Bytes::copy_from_slice(bytes));
            state.waker.take()
        };
        if let Some(wake) = wake { wake.wake(); }
    }
    pub fn eof(&self) {
        let wake = {
            let mut state = lock(&self.0.control);
            state.eof = true;
            state.waker.take()
        };
        if let Some(wake) = wake { wake.wake(); }
    }
}

struct FakeBody(Probe);

impl Drop for FakeBody {
    fn drop(&mut self) {
        self.0.0.drops.fetch_add(1, Ordering::SeqCst);
        let mut state = lock(&self.0.0.control);
        state.closed = true;
        state.chunks.clear();
        state.waker = None;
        state.generated = None;
    }
}

impl Stream for FakeBody {
    type Item = Result<Bytes, reqwest::Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let probe = &self.0;
        probe.0.polls.fetch_add(1, Ordering::SeqCst);
        let mut state = lock(&probe.0.control);
        let bytes = if let Some(config) = &state.generated {
            let (account, count, padding, cooperate) =
                (config.account, config.count, config.padding, config.cooperate);
            if state.yield_next {
                state.yield_next = false;
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            if state.next < count {
                let index = state.next;
                state.next += 1;
                state.yield_next = cooperate;
                probe.0.generated.fetch_add(1, Ordering::SeqCst);
                Bytes::from(format!("data:account={account};seq={index};{}\n\n", "x".repeat(padding)))
            } else if !state.terminal_sent {
                state.terminal_sent = true;
                Bytes::from(format!("data:account={account};done\n\n"))
            } else {
                return Poll::Ready(None);
            }
        } else if let Some(bytes) = state.chunks.pop_front() {
            bytes
        } else if state.eof {
            return Poll::Ready(None);
        } else {
            state.waker = Some(cx.waker().clone());
            return Poll::Pending;
        };
        probe.0.max_chunk.fetch_max(bytes.len(), Ordering::SeqCst);
        Poll::Ready(Some(Ok(bytes)))
    }
}

#[derive(Clone, Copy)]
pub struct BodyLimits {
    pub total: usize,
    pub incoming: usize,
    pub emitted: usize,
}

impl Default for BodyLimits {
    fn default() -> Self {
        Self { total: 2 * 1024 * 1024, incoming: 2048, emitted: 128 }
    }
}

pub fn response(
    status: u16,
    content_type: Option<&str>,
    usage: Option<&str>,
    generated: Option<Generated>,
    limits: BodyLimits,
) -> (NativeResponse, NativeCancellation, Probe) {
    assert!(limits.emitted > 0 && limits.incoming >= limits.emitted);
    let active = Arc::new(AtomicUsize::new(0));
    let probe = Probe(Arc::new(ProbeInner {
        control: Mutex::new(Control {
            chunks: VecDeque::new(), eof: false, closed: false, waker: None,
            generated, next: 0, terminal_sent: false, yield_next: false,
        }),
        polls: AtomicUsize::new(0), drops: AtomicUsize::new(0),
        generated: AtomicUsize::new(0), max_chunk: AtomicUsize::new(0),
        active: Arc::clone(&active), stream: Mutex::new(None),
    }));
    let cancellation = NativeCancellation::new();
    cancellation.claim().unwrap();
    let state = Arc::new(Mutex::new(StreamingState {
        body: Some(Box::pin(FakeBody(probe.clone()))),
        pending: Bytes::new(),
        permit: Some(Permit::acquire(&active, 1).unwrap()),
        received: 0, limit: limits.total, incoming_limit: limits.incoming,
        emitted_limit: limits.emitted, finished: false, error: None,
        closure: None,
    }));
    *lock(&cancellation.inner.response) = Some(Arc::downgrade(&state));
    *lock(&probe.0.stream) = Some(Arc::downgrade(&state));
    let body = NativeByteStream { state, cancellation: cancellation.clone() };
    (NativeResponse {
        status, usage_header: usage.map(str::to_owned),
        content_type: content_type.map(str::to_owned), body,
    }, cancellation, probe)
}
