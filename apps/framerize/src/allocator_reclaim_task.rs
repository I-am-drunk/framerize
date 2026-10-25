//! Opt-in pressure maintenance. All allocator and filesystem work is blocking.
use crate::allocator_reclaim::{Reclaimer, FLAG, SUPPORTED};
use framer_core::observability::Event;
use std::time::Duration;
use tokio::task::{JoinError, JoinHandle};

pub struct Task(JoinHandle<()>);

impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub fn start() -> Option<Task> {
    if !SUPPORTED || std::env::var(FLAG).ok().as_deref() != Some("1") {
        return None;
    }
    Some(Task(tokio::spawn(async {
        let mut reclaimer = Reclaimer::default();
        loop {
            // Sleep after each completed pass. Missed ticks never catch up in
            // a burst, and at most one blocking pass can exist for this task.
            tokio::time::sleep(Duration::from_secs(30)).await;
            match blocking_tick(reclaimer, collect).await {
                Ok(state) => reclaimer = state,
                Err(_) => {
                    eprintln!("Rust memory maintenance stopped after a worker failure");
                    break;
                }
            }
        }
    })))
}

async fn blocking_tick<T, F>(mut state: T, tick: F) -> Result<T, JoinError>
where
    T: Send + 'static,
    F: FnOnce(&mut T) + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        tick(&mut state);
        state
    }).await
}

fn collect(reclaimer: &mut Reclaimer) {
    if let Some(result) = reclaimer.poll() {
        // Counts only. No account identity, buffer contents or request values.
        Event::new("memory.rust_reclaim")
            .duration_ms(result.elapsed.as_millis())
            .field("rss_before_bytes", result.before_rss)
            .field("rss_after_bytes", result.after_rss)
            .field("free_heap_bytes", result.free_heap)
            .field("allocator_reported_release", result.allocator_reported_release)
            .outcome("ok")
            .emit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_tick_keeps_async_work_responsive_and_preserves_state() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let work = tokio::spawn(blocking_tick(41u64, move |state| {
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            *state += 1;
        }));
        tokio::time::timeout(Duration::from_secs(1), started_rx)
            .await.unwrap().unwrap();
        // The blocking worker is still waiting. A current-thread executor
        // must be free to run this task before the worker can be released.
        assert_eq!(tokio::spawn(async { 7 }).await.unwrap(), 7);
        release_tx.send(()).unwrap();
        assert_eq!(work.await.unwrap().unwrap(), 42);
    }
}
