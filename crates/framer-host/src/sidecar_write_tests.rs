use super::write_command;
use serde_json::json;
use std::alloc::{GlobalAlloc, Layout, System};
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::AsyncWrite;

struct WatchedAllocator;
static INPUT_PTR: AtomicUsize = AtomicUsize::new(0);
static WIRE_PTR: AtomicUsize = AtomicUsize::new(0);
static INPUT_FREED: AtomicUsize = AtomicUsize::new(0);
static WIRE_FREED: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static ALLOCATOR: WatchedAllocator = WatchedAllocator;

// Other test allocations pass directly through to System.
unsafe impl GlobalAlloc for WatchedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let address = ptr as usize;
        if INPUT_PTR.compare_exchange(address, 0, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            INPUT_FREED.store(layout.size(), Ordering::SeqCst);
        }
        if WIRE_PTR.compare_exchange(address, 0, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            WIRE_FREED.store(layout.size(), Ordering::SeqCst);
        }
        unsafe { System.dealloc(ptr, layout) }
    }
}

struct GatedWriter {
    state: Arc<AtomicU8>,
    bytes: Vec<u8>,
    track: bool,
    observed: bool,
}

impl AsyncWrite for GatedWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.track && !self.observed {
            // Check ownership at the first attempted write to a full pipe.
            assert_eq!(INPUT_PTR.load(Ordering::SeqCst), 0, "decoded input still owned");
            assert_ne!(INPUT_FREED.load(Ordering::SeqCst), 0);
            WIRE_PTR.store(buf.as_ptr() as usize, Ordering::SeqCst);
            self.observed = true;
        }
        match self.state.load(Ordering::SeqCst) {
            0 => Poll::Pending,
            2 => Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, "fixture"))),
            _ => {
                // Short writes include splits inside UTF-8 sequences.
                let count = buf.len().min(257);
                self.bytes.extend_from_slice(&buf[..count]);
                Poll::Ready(Ok(count))
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[test]
fn command_releases_buffers_on_success_error_and_cancellation() {
    const PAYLOAD_SIZE: usize = 2 * 1024 * 1024;
    let mut cx = Context::from_waker(futures::task::noop_waker_ref());
    for outcome in [1, 2, 0] {
        let fields = json!({"id": 19, "cmd": "fixture", "body": "x".repeat(PAYLOAD_SIZE)});
        INPUT_FREED.store(0, Ordering::SeqCst);
        WIRE_FREED.store(0, Ordering::SeqCst);
        INPUT_PTR.store(fields["body"].as_str().unwrap().as_ptr() as usize, Ordering::SeqCst);
        let state = Arc::new(AtomicU8::new(0));
        let mut writer = GatedWriter {
            state: state.clone(), bytes: Vec::new(), track: true, observed: false,
        };
        let mut future = Box::pin(write_command(&mut writer, fields));
        assert!(future.as_mut().poll(&mut cx).is_pending());
        assert!(INPUT_FREED.load(Ordering::SeqCst) >= PAYLOAD_SIZE);
        assert_eq!(WIRE_FREED.load(Ordering::SeqCst), 0);
        if outcome != 0 {
            state.store(outcome, Ordering::SeqCst);
            let result = match future.as_mut().poll(&mut cx) {
                Poll::Ready(result) => result,
                Poll::Pending => panic!("fixture writer should finish"),
            };
            if outcome == 1 {
                assert!(result.is_ok());
            } else {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
            }
            // No wire buffer remains when the caller starts waiting for a reply.
            assert!(WIRE_FREED.load(Ordering::SeqCst) > PAYLOAD_SIZE);
        }
        drop(future);
        assert_eq!(WIRE_PTR.load(Ordering::SeqCst), 0);
        assert!(WIRE_FREED.load(Ordering::SeqCst) > PAYLOAD_SIZE);
        if outcome == 1 {
            let decoded: serde_json::Value = serde_json::from_slice(&writer.bytes).unwrap();
            assert_eq!(decoded["id"], 19);
            assert_eq!(decoded["body"].as_str().unwrap().len(), PAYLOAD_SIZE);
            assert_eq!(writer.bytes.last(), Some(&b'\n'));
        } else {
            assert!(writer.bytes.is_empty());
        }
    }
}

#[test]
fn command_preserves_exact_wire_bytes_across_short_writes() {
    let mut cx = Context::from_waker(futures::task::noop_waker_ref());
    for fields in [
        json!({"id": 1, "cmd": "fixture", "nested": [true, null, 1.25, {"x": "line\n\t\u{0}\"\\ café 😀"}]}),
        json!({"id": 2, "body": "日本語 😀\n".repeat(8192)}),
        json!({"id": 3, "body": ["one", "two", "three"]}),
    ] {
        let expected = format!("{}\n", fields).into_bytes();
        let mut writer = GatedWriter {
            state: Arc::new(AtomicU8::new(1)), bytes: Vec::new(), track: false, observed: false,
        };
        let mut future = Box::pin(write_command(&mut writer, fields));
        assert!(matches!(future.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
        drop(future);
        assert_eq!(writer.bytes, expected);
    }
}
