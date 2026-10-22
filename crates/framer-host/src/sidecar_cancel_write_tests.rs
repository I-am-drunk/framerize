use super::write_owned_command;
use serde_json::json;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;
use tokio::io::AsyncWrite;
use tokio::sync::{Mutex, Notify};

struct State {
    limit: usize,
    bytes: Vec<u8>,
    waker: Option<Waker>,
    failed: bool,
}

struct Pipe {
    state: Arc<StdMutex<State>>,
    progress: Arc<Notify>,
}

impl AsyncWrite for Pipe {
    fn poll_write(
        self: Pin<&mut Self>, cx: &mut Context<'_>, bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut state = self.state.lock().unwrap();
        if state.failed {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, "fixture")));
        }
        let available = state.limit.saturating_sub(state.bytes.len());
        if available == 0 {
            state.waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let n = bytes.len().min(available);
        state.bytes.extend_from_slice(&bytes[..n]);
        drop(state);
        self.progress.notify_one();
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

fn pipe(limit: usize) -> (Arc<Mutex<Pipe>>, Arc<StdMutex<State>>, Arc<Notify>) {
    let state = Arc::new(StdMutex::new(State {
        limit, bytes: Vec::new(), waker: None, failed: false,
    }));
    let progress = Arc::new(Notify::new());
    let writer = Arc::new(Mutex::new(Pipe {
        state: state.clone(), progress: progress.clone(),
    }));
    (writer, state, progress)
}

fn release(state: &StdMutex<State>) {
    let wake = {
        let mut state = state.lock().unwrap();
        state.limit = usize::MAX;
        state.waker.take()
    };
    if let Some(waker) = wake {
        waker.wake();
    }
}

#[tokio::test]
async fn cancelled_queued_command_is_never_written() {
    let (writer, state, _) = pipe(usize::MAX);
    let guard = writer.lock().await;
    {
        let queued = write_owned_command(writer.clone(), json!({"id": 1, "body": "cancelled"}));
        tokio::pin!(queued);
        assert!(futures::poll!(queued.as_mut()).is_pending());
    }
    drop(guard);
    let next = json!({"id": 2, "body": "healthy"});
    let expected = format!("{next}\n");
    tokio::time::timeout(Duration::from_secs(2), write_owned_command(writer, next))
        .await.unwrap().unwrap();
    assert_eq!(state.lock().unwrap().bytes, expected.as_bytes());
}

#[tokio::test]
async fn cancellation_after_a_short_write_preserves_the_next_frame() {
    let (writer, state, progress) = pipe(11);
    let first = json!({"id": 1, "body": "partial café 日本語 😀"});
    let second = json!({"id": 2, "body": "healthy"});
    let expected = format!("{first}\n{second}\n");
    let caller = tokio::spawn(write_owned_command(writer.clone(), first));
    tokio::time::timeout(Duration::from_secs(2), progress.notified()).await.unwrap();
    assert_eq!(state.lock().unwrap().bytes.len(), 11);
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let healthy = tokio::spawn(write_owned_command(writer, second));
    release(&state);
    tokio::time::timeout(Duration::from_secs(2), healthy)
        .await.unwrap().unwrap().unwrap();
    let bytes = state.lock().unwrap().bytes.clone();
    assert_eq!(bytes, expected.as_bytes());
    let lines: Vec<_> = bytes.split(|b| *b == b'\n').filter(|s| !s.is_empty()).collect();
    assert_eq!(lines.len(), 2);
    for line in lines {
        serde_json::from_slice::<serde_json::Value>(line).unwrap();
    }
}

#[tokio::test]
async fn owned_writer_preserves_io_errors() {
    let (writer, state, _) = pipe(usize::MAX);
    state.lock().unwrap().failed = true;
    let error = tokio::time::timeout(
        Duration::from_secs(2), write_owned_command(writer, json!({"id": 3})),
    ).await.unwrap().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert!(state.lock().unwrap().bytes.is_empty());
}
