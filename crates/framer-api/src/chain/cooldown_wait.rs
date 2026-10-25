use framer_core::{error::AccountCooldown, port::ChainConfig};
use std::{future::Future, time::Duration};
use tokio::time::Instant;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};

    #[tokio::test(start_paused = true)]
    async fn cooldown_never_retries_early_and_cancellation_stops_waiting() {
        for cancel in [false, true] {
            let flag = Arc::new(AtomicBool::new(false));
            let signal = flag.clone();
            let cfg = ChainConfig { cancelled:Some(Arc::new(move || signal.load(Ordering::SeqCst))),
                ..ChainConfig::default() };
            let calls = AtomicUsize::new(0);
            let mut wait = CooldownWait::default();
            let timer = wait.timer();
            {
                let future = wait.run(&cfg, true, || async {
                    if calls.fetch_add(1, Ordering::SeqCst) == 0 { Err("account-cooling:600".into()) }
                    else { Ok(()) }
                });
                tokio::pin!(future);
                assert!(futures::poll!(&mut future).is_pending());
                tokio::time::advance(Duration::from_secs(599)).await;
                assert!(futures::poll!(&mut future).is_pending());
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                flag.store(cancel, Ordering::SeqCst);
                let result = future.await;
                assert_eq!(result.is_err(), cancel);
                assert_eq!(calls.load(Ordering::SeqCst), if cancel { 1 } else { 2 });
            }
            assert_eq!(wait.elapsed(timer), Duration::ZERO);
            assert_eq!(wait.spent.as_secs(), if cancel { 599 } else { 600 });
        }
    }

    #[tokio::test(start_paused = true)]
    async fn cumulative_wait_is_bounded_and_generation_time_is_preserved() {
        let mut wait = CooldownWait::default();
        let timer = wait.timer();
        for (seconds, retries) in [(200, 2), (400, 2), (1, 1)] {
            let calls = AtomicUsize::new(0);
            let result = wait.run(&ChainConfig::default(), true, || async {
                tokio::time::sleep(Duration::from_secs(2)).await;
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(format!("account-cooling:{seconds}"))
                } else { Ok(()) }
            }).await;
            assert_eq!(result.is_ok(), retries == 2);
            assert_eq!(calls.load(Ordering::SeqCst), retries);
        }
        assert_eq!(wait.spent, Duration::from_secs(600));
        assert_eq!(wait.elapsed(timer), Duration::from_secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn a_renewed_cooldown_returns_without_repeated_waits() {
        let mut wait = CooldownWait::default();
        let calls = AtomicUsize::new(0);
        let result: Result<(), String> = wait.run(&ChainConfig::default(), true, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err("account-cooling:10".into())
        }).await;
        assert_eq!(result.unwrap_err(), "account-cooling:10");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(wait.spent, Duration::from_secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn first_chunk_and_nonlocal_errors_never_wait_or_retry() {
        for (has_output, error) in [(false, "account-cooling:10"),
            (true, "account-cooling:601"), (true, "risk-refused"),
            (true, "upstream 403: account-cooling:10"), (true, "account-cooling:bad")] {
            let mut wait = CooldownWait::default();
            let started = Instant::now();
            let calls = AtomicUsize::new(0);
            let result: Result<(), String> = wait.run(&ChainConfig::default(), has_output, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(error.into())
            }).await;
            assert_eq!(result.unwrap_err(), error);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(started.elapsed(), Duration::ZERO);
        }
    }
}

/// Preserve a partly delivered generation across a local capacity wait.
/// At most ten minutes of waiting is allowed across the entire chain.
#[derive(Default)]
pub(crate) struct CooldownWait {
    spent: Duration,
}

#[derive(Clone, Copy)]
pub(crate) struct Timer {
    started: Instant,
    waiting: Duration,
}

impl CooldownWait {
    async fn wait(&mut self, cfg: &ChainConfig, duration: Duration) -> Result<(), String> {
        let started = Instant::now();
        let deadline = started + duration;
        let result = loop {
            if let Err(error) = cfg.check_cancelled() {
                break Err(error);
            }
            if Instant::now() >= deadline {
                break Ok(());
            }
            tokio::time::sleep_until(deadline.min(Instant::now() + Duration::from_millis(100))).await;
        };
        self.spent += started.elapsed();
        result
    }

    pub(crate) fn timer(&self) -> Timer {
        Timer { started: Instant::now(), waiting: self.spent }
    }

    pub(crate) fn elapsed(&self, timer: Timer) -> Duration {
        timer.started.elapsed().saturating_sub(self.spent.saturating_sub(timer.waiting))
    }

    pub(crate) async fn run<T, F, Fut>(&mut self, cfg: &ChainConfig,
        has_output: bool, mut attempt: F) -> Result<T, String>
    where F: FnMut() -> Fut, Fut: Future<Output = Result<T, String>> {
        cfg.check_cancelled()?;
        let error = match attempt().await {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        let Some(cooldown) = AccountCooldown::from_error(&error).filter(|_| has_output) else {
            return Err(error);
        };
        let wait = Duration::from_secs(cooldown.retry_after_secs.max(1));
        if wait > Duration::from_secs(600).saturating_sub(self.spent) {
            return Err(error);
        }
        self.wait(cfg, wait).await?;
        // Only the locally refused attempt is retried, once, with the same input.
        cfg.check_cancelled()?;
        attempt().await
    }
}
