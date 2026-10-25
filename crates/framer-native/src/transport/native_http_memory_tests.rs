use super::*;
use std::collections::VecDeque;

struct Storage { data: Box<[u8]>, live: Arc<AtomicUsize> }
impl AsRef<[u8]> for Storage { fn as_ref(&self) -> &[u8] { &self.data } }
impl Drop for Storage {
    fn drop(&mut self) { self.live.fetch_sub(self.data.len(), Ordering::SeqCst); }
}

fn hidden_backing(payload: &[u8]) -> (Bytes, Arc<AtomicUsize>) {
    let size = 1024 * 1024;
    let mut data = vec![0; size].into_boxed_slice();
    data[17..17 + payload.len()].copy_from_slice(payload);
    let live = Arc::new(AtomicUsize::new(size));
    let bytes = Bytes::from_owner(Storage { data, live: live.clone() });
    (bytes.slice(17..17 + payload.len()), live)
}

struct Wire {
    chunks: VecDeque<Bytes>, eof: bool, polls: Arc<AtomicUsize>, drops: Arc<AtomicUsize>,
}
impl Stream for Wire {
    type Item = Result<Bytes, reqwest::Error>;
    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        if let Some(bytes) = self.chunks.pop_front() { Poll::Ready(Some(Ok(bytes))) }
        else if self.eof { Poll::Ready(None) }
        else { Poll::Pending }
    }
}
impl Drop for Wire { fn drop(&mut self) { self.drops.fetch_add(1, Ordering::SeqCst); } }

struct Fixture {
    body: NativeByteStream, cancel: NativeCancellation,
    polls: Arc<AtomicUsize>, drops: Arc<AtomicUsize>, active: Arc<AtomicUsize>,
}
fn stream(chunks: Vec<Bytes>, incoming: usize, emitted: usize, total: usize, eof: bool) -> Fixture {
    let polls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let cancel = NativeCancellation::new();
    cancel.claim().unwrap();
    let state = Arc::new(Mutex::new(StreamingState {
        body: Some(Box::pin(Wire { chunks: chunks.into(), eof, polls: polls.clone(), drops: drops.clone() })),
        pending: Bytes::new(), permit: Some(Permit::acquire(&active, 1).unwrap()),
        received: 0, limit: total, incoming_limit: incoming, emitted_limit: emitted,
        finished: false, error: None, closure: None,
    }));
    *lock(&cancel.inner.response) = Some(Arc::downgrade(&state));
    Fixture { body: NativeByteStream { state, cancellation: cancel.clone() }, cancel, polls, drops, active }
}
fn poll(body: &mut NativeByteStream) -> Poll<Option<Result<Bytes, String>>> {
    let waker = futures::task::noop_waker();
    Pin::new(body).poll_next(&mut Context::from_waker(&waker))
}
fn chunk(body: &mut NativeByteStream) -> Bytes {
    match poll(body) { Poll::Ready(Some(Ok(bytes))) => bytes, _ => panic!("expected a fixture chunk") }
}

#[test]
fn small_wire_slice_releases_large_owner_before_delivery() {
    let (input, live) = hidden_backing(b"abcdefgh");
    let mut f = stream(vec![input], 16, 4, 32, false);
    assert_eq!(live.load(Ordering::SeqCst), 1024 * 1024);
    let first = chunk(&mut f.body);
    assert_eq!(&first[..], b"abcd");
    assert_eq!(live.load(Ordering::SeqCst), 0);
    assert_eq!(lock(&f.body.state).pending.len(), 4);
    assert_eq!(f.polls.load(Ordering::SeqCst), 1);
    assert_eq!(f.active.load(Ordering::SeqCst), 1);
}

#[test]
fn final_slice_does_not_share_an_unpolled_empty_remainder() {
    let mut f = stream(vec![Bytes::from(vec![7u8; 8])], 16, 4, 32, false);
    drop(chunk(&mut f.body));
    let last = chunk(&mut f.body);
    assert_eq!(&last[..], &[7u8; 4]);
    assert!(lock(&f.body.state).pending.is_empty());
    assert!(last.try_into_mut().is_ok(), "the idle body must not retain an empty shared slice");
    assert_eq!(f.polls.load(Ordering::SeqCst), 1);
    assert_eq!(f.active.load(Ordering::SeqCst), 1);
}

#[test]
fn over_limit_wire_chunk_closes_without_retaining_source() {
    let (input, live) = hidden_backing(b"seventeen bytes!!");
    let mut f = stream(vec![input], 8, 4, 32, false);
    assert!(matches!(poll(&mut f.body), Poll::Ready(Some(Err(e))) if e.contains("transport chunk")));
    assert_eq!(live.load(Ordering::SeqCst), 0);
    assert_eq!(lock(&f.body.state).received, 0);
    assert_eq!(f.drops.load(Ordering::SeqCst), 1);
    assert_eq!(f.active.load(Ordering::SeqCst), 0);
    assert!(matches!(poll(&mut f.body), Poll::Ready(None)));
}

#[test]
fn total_limit_preserves_prefix_then_releases_next_source() {
    let (first, live1) = hidden_backing(b"abcdefgh");
    let (second, live2) = hidden_backing(b"ijklmnop");
    let mut f = stream(vec![first, second], 16, 8, 12, false);
    let prefix = chunk(&mut f.body);
    assert_eq!(&prefix[..], b"abcdefgh");
    assert_eq!(live1.load(Ordering::SeqCst), 0);
    assert!(matches!(poll(&mut f.body), Poll::Ready(Some(Err(e))) if e == RESPONSE_LIMIT));
    assert_eq!(live2.load(Ordering::SeqCst), 0);
    assert_eq!(lock(&f.body.state).received, 8);
    assert_eq!(f.active.load(Ordering::SeqCst), 0);
    assert_eq!(&prefix[..], b"abcdefgh");
}

#[test]
fn cancellation_releases_pending_without_body_polling() {
    let (input, live) = hidden_backing(b"abcdefgh");
    let mut f = stream(vec![input], 16, 2, 32, false);
    let emitted = chunk(&mut f.body);
    let retained = emitted.clone();
    assert_eq!(live.load(Ordering::SeqCst), 0);
    f.cancel.cancel();
    assert!(lock(&f.body.state).pending.is_empty());
    assert_eq!(f.active.load(Ordering::SeqCst), 0);
    assert_eq!(f.drops.load(Ordering::SeqCst), 1);
    assert_eq!(f.polls.load(Ordering::SeqCst), 1);
    assert_eq!(&retained[..], b"ab");
    assert!(matches!(poll(&mut f.body), Poll::Ready(Some(Err(e))) if e == CANCELLED));
    assert!(matches!(poll(&mut f.body), Poll::Ready(None)));
}

#[test]
fn normalization_preserves_binary_utf8_crlf_and_empty_chunks() {
    let expected = b"data: \xf0\x9f\x92\xa1\r\n\r\n\0\xff";
    let slices: Vec<Bytes> = vec![expected[..8].to_vec().into(), Bytes::new(),
        expected[8..10].to_vec().into(), expected[10..].to_vec().into()];
    let mut f = stream(slices, 16, 3, 64, true);
    let mut actual = Vec::new();
    loop {
        match poll(&mut f.body) {
            Poll::Ready(Some(Ok(bytes))) => { assert!(bytes.len() <= 3); actual.extend_from_slice(&bytes); }
            Poll::Ready(None) => break,
            _ => panic!("unexpected fixture state"),
        }
    }
    assert_eq!(actual, expected);
    assert_eq!(f.active.load(Ordering::SeqCst), 0);
}

#[test]
fn ten_bodies_release_source_backings_and_cancel_independently() {
    let mut fixtures = Vec::new();
    let mut held = Vec::new();
    for account in 0..10u8 {
        let (input, live) = hidden_backing(&[account; 16]);
        let mut f = stream(vec![input], 16, 4, 32, false);
        held.push(chunk(&mut f.body));
        assert_eq!(live.load(Ordering::SeqCst), 0);
        fixtures.push(f);
    }
    fixtures[0].cancel.cancel();
    assert_eq!(fixtures[0].active.load(Ordering::SeqCst), 0);
    for account in 1..10 {
        assert_eq!(fixtures[account].active.load(Ordering::SeqCst), 1);
        assert_eq!(&chunk(&mut fixtures[account].body)[..], &[account as u8; 4]);
    }
    for (account, bytes) in held.into_iter().enumerate() {
        assert_eq!(&bytes[..], &[account as u8; 4]);
    }
    for f in &fixtures { f.cancel.cancel(); assert_eq!(f.active.load(Ordering::SeqCst), 0); }
}
